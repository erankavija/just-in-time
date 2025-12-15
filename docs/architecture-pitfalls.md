# Architecture and Common Pitfalls

This document explains non-obvious architecture decisions and common pitfalls to help contributors avoid confusion.

## Module Declaration: Binary vs Library

### ⚠️ Current Architecture (Before Phase I Refactoring)

**Problem:** The binary (`main.rs`) re-declares all modules instead of using the library.

```rust
// main.rs
mod cli;        // ❌ Duplicates lib.rs
mod commands;   // ❌ Duplicates lib.rs  
mod config;     // ❌ Duplicates lib.rs
// ... all other modules

// lib.rs
pub mod cli;
pub mod commands;
pub mod config;
// ... same modules declared again
```

**Why this is confusing:**
1. Each module is compiled **twice** (once for lib, once for bin)
2. `crate::config` means different things in different contexts
3. Adding a new module requires changes in **TWO files**
4. Non-standard Rust pattern (most projects use lib from bin)

### Adding a New Module (Current Process)

**IMPORTANT:** Until Phase I refactoring is complete, you must:

1. Add `pub mod your_module;` to `lib.rs`
2. Add `mod your_module;` to `main.rs` ← **CRITICAL: Don't forget this!**
3. Use `crate::your_module::...` in submodules

**Example:**
```rust
// lib.rs
pub mod my_feature;  // Step 1

// main.rs
mod my_feature;      // Step 2 ← REQUIRED or binary won't compile!

// commands/something.rs
use crate::my_feature::MyType;  // Step 3
```

### After Phase I Refactoring (Future State)

The binary will use the library normally:

```rust
// main.rs (after refactoring)
use jit::my_feature::MyType;  // ✅ Standard Rust pattern
```

**Adding new modules will only need one place:**
1. Add `pub mod your_module;` to `lib.rs`
2. Done! Binary automatically gets it via `use jit::...`

## Import Path Confusion

### Problem: `crate::` vs `jit::`

**Before Phase I:**
- In `lib.rs` and submodules: `crate::config` refers to the library
- In `main.rs`: `crate::config` refers to main.rs's module tree
- This is **different** depending on context!

**After Phase I:**
- `crate::` always means the library (consistent)
- `jit::` is the external crate name (for imports in main.rs)

### Debugging Import Errors

If you see:
```
error[E0432]: unresolved import `crate::my_module`
   |
   | use crate::my_module::Thing;
   |            ^^^^^^^^^
   |            unresolved import
   |            help: a similar path exists: `jit::my_module`
```

**Current fix (before Phase I):**
1. Check if `mod my_module;` exists in BOTH `lib.rs` and `main.rs`
2. If missing from `main.rs`, add it there
3. Rebuild

**Future fix (after Phase I):**
1. Check if `pub mod my_module;` exists in `lib.rs`
2. Use `use jit::my_module::Thing;` in main.rs
3. Done

## Common Pitfalls

### Pitfall 1: Forgetting to Add Module to main.rs

**Symptom:**
```
error[E0432]: unresolved import `crate::config`
```

**Why:** Binary compiles its own module tree, doesn't see lib.rs modules

**Fix:** Add `mod config;` to main.rs (until Phase I complete)

### Pitfall 2: IssueStore Trait Missing Methods

**Symptom:**
```
error[E0599]: no method named `root` found for type parameter `S`
```

**Why:** Trait methods must be in scope via trait import

**Fix:** 
```rust
use crate::storage::IssueStore;  // Brings trait methods into scope
```

### Pitfall 3: Wrong Argument Order in create_issue()

**Symptom:** Type errors about `Priority` vs `Vec<String>`

**Current signature:**
```rust
fn create_issue(
    title: String,
    description: String,
    priority: Priority,     // ← 3rd parameter
    gates: Vec<String>,     // ← 4th parameter  
    labels: Vec<String>,    // ← 5th parameter
) -> Result<String>
```

**Common mistake:**
```rust
// Wrong:
executor.create_issue(
    "Title".to_string(),
    "Desc".to_string(),
    vec!["type:task".to_string()],  // ← This is labels, not priority!
    Priority::Normal,
    Vec::new(),
)?;

// Correct:
executor.create_issue(
    "Title".to_string(),
    "Desc".to_string(),
    Priority::Normal,               // ← Priority is 3rd
    Vec::new(),                     // ← Gates are 4th
    vec!["type:task".to_string()],  // ← Labels are 5th
)?;
```

**Note:** This will be standardized in v1.0 (breaking change)

### Pitfall 4: JsonFileStorage::new() Doesn't Return Result

**Symptom:** `.unwrap()` doesn't work on `JsonFileStorage::new()`

**Why:** Constructor never fails, path validation happens on first use

**Fix:**
```rust
// Wrong:
let storage = JsonFileStorage::new(path).unwrap();  // ❌ No unwrap needed

// Correct:
let storage = JsonFileStorage::new(path);           // ✅ Returns Self directly
storage.init()?;                                    // ✅ Validate with init()
```

**Note:** `try_new()` variant may be added in future for immediate validation

## Testing Patterns

### Use InMemoryStorage for Fast Tests

```rust
use jit::storage::InMemoryStorage;

#[test]
fn test_something() {
    let storage = InMemoryStorage::new();  // ✅ 10-100x faster than JSON
    storage.init().unwrap();
    
    let executor = CommandExecutor::new(storage);
    // ... test logic
}
```

### Test Config Loading with TempDir

```rust
use tempfile::TempDir;
use jit::storage::{IssueStore, JsonFileStorage};

#[test]
fn test_config() {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());
    
    // Write config
    std::fs::write(storage.root().join("config.toml"), "...").unwrap();
    
    // Test config loading
    let config = JitConfig::load(storage.root()).unwrap();
    // ... assertions
}
```

## Performance Notes

### Config Loading is On-Demand

Currently, config is loaded from disk on every `check_warnings()` call. This is intentional:
- **Pros:** Simple, no caching complexity, always fresh
- **Cons:** Minor I/O overhead (typically <1ms for small config)

**When to optimize:** If profiling shows config loading is a bottleneck (unlikely)

**How to optimize:** Add `config_cache: Option<Arc<JitConfig>>` to CommandExecutor

### Compilation Time

**Before Phase I refactoring:**
- All modules compiled twice (lib + bin)
- Typical debug build: ~8-10 seconds

**After Phase I refactoring:**
- Modules compiled once
- Expected improvement: 5-10% faster builds

## Module Organization

### Where to Put New Code

**Core domain logic** → `src/domain/`
- Data structures: Issue, State, Priority, Gate
- Pure functions: validation, transformations

**Business logic** → `src/commands/`
- CommandExecutor methods
- Organize by feature area (issue.rs, gate.rs, validate.rs)

**Storage** → `src/storage/`
- IssueStore trait implementations
- File I/O, locking, persistence

**CLI interface** → `src/cli/`
- Clap command definitions
- Argument parsing

**Binary-specific** → `src/main.rs` or modules declared there
- Entry point logic
- Exit code handling
- Currently: output_macros (uses println!)

## When in Doubt

1. **Check existing patterns:** Look at similar modules
2. **Run tests:** `cargo test` catches most issues
3. **Check clippy:** `cargo clippy` for best practices
4. **Review docs:** This file, ROADMAP.md, design.md
5. **Ask:** Create an issue or PR for clarification

## Related Documents

- `docs/refactoring-plan-binary-to-library.md` - Detailed plan to fix main architecture issue
- `docs/session-2025-12-15-phase-h-implementation.md` - Context on confusion encountered
- `copilot-instructions.md` - Coding standards and best practices
- `ROADMAP.md` - Feature priorities and completed work

---

**Last updated:** 2025-12-15 (Phase H complete, Phase I planned)  
**Status:** This document describes **current** architecture. Phase I will simplify significantly.
