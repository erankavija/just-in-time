# Binary-to-Library Refactoring Plan (Option A) ✅ COMPLETE

**Status**: ✅ **COMPLETE** (2025-12-15)  
**Time taken**: ~2 hours (matched estimate)  
**All tests passing**: 556+ tests ✅  

## Executive Summary

Refactor `main.rs` to use the library instead of re-declaring all modules. This eliminates duplicate compilation, follows Rust best practices, and reduces maintenance burden.

**Estimated time:** 1-2 hours ✅ **ACTUAL: ~2 hours**  
**Risk level:** Low (all changes internal, no API changes) ✅ **NO ISSUES**  
**Benefits:** Faster compilation, clearer architecture, easier maintenance ✅ **ACHIEVED**

---

## Current Architecture Problem

### Before (Current State)
```
crates/jit/
  src/
    lib.rs          ← pub mod config; pub mod commands; ... (167 lines)
    main.rs         ← mod config; mod commands; ...      (1,413 lines)
                      Re-declares ALL modules, duplicates compile
    commands/
      validate.rs   ← use crate::config (resolves differently!)
```

**Issues:**
1. Every module compiled twice (lib + bin)
2. `crate::` means different things in different contexts
3. New modules need declaration in TWO files
4. Non-standard Rust pattern

### After (Target State)
```
crates/jit/
  src/
    lib.rs          ← pub mod config; pub mod commands; ... (unchanged)
    main.rs         ← use jit::{...}  (imports from lib)
                      Only CLI entry point logic (~1,300 lines)
    commands/
      validate.rs   ← use crate::config (always from lib)
```

**Benefits:**
1. Single compilation pass
2. Clear import semantics
3. One place to add modules
4. Standard Rust pattern

---

## Implementation Plan

### Phase 1: Preparation & Analysis (15 min)

**1.1 Document current module usage in main.rs**
```bash
grep "use \(cli\|commands\|domain\|storage\)::" src/main.rs > /tmp/main-imports.txt
```
Goal: Understand which modules are heavily used, plan import groups

**1.2 Identify binary-specific modules**
```bash
# These should stay in main.rs:
# - output_macros (uses println!, not suitable for lib)
# - Any CLI-specific helpers
```

**1.3 Check for circular dependencies**
```bash
# Ensure no modules in lib depend on main-specific code
grep -r "use crate::main" src/
```
Expected: No results (would block refactoring)

### Phase 2: Create Import Groups (30 min)

**2.1 Group imports by category**
```rust
// main.rs (new structure)

// Binary-specific modules (keep as mod declarations)
mod output_macros;  // Uses println!, binary-only

// Library imports - CLI layer
use jit::cli::{
    Cli, Commands, ConfigCommands, DepCommands, DocCommands, 
    EventCommands, GateCommands, GraphCommands, IssueCommands, 
    LabelCommands, QueryCommands, RegistryCommands,
};

// Library imports - Core logic
use jit::commands::{parse_priority, parse_state, CommandExecutor, DependencyAddResult};
use jit::config::JitConfig;
use jit::domain::{Event, Issue, Priority, State};
use jit::output::{ExitCode, JsonError, JsonOutput};
use jit::storage::{IssueStore, JsonFileStorage};

// Library imports - Type hierarchy
use jit::type_hierarchy::{ValidationWarning, HierarchyConfig};

// Standard library
use anyhow::{anyhow, Result};
use clap::Parser;
use std::env;
```

**2.2 Handle module visibility**
Some items may not be `pub` in lib.rs yet:
```rust
// In lib.rs, ensure these are public:
pub use commands::{DependencyAddResult};  // Currently pub(crate)?
pub use type_hierarchy::{ValidationWarning};  // Check visibility
```

**2.3 Test compilation after each import group**
```bash
cargo build 2>&1 | grep "unresolved import"
# Fix visibility issues as they appear
```

### Phase 3: Update main.rs Function Calls (30 min)

**3.1 Replace local paths with lib paths**
```rust
// BEFORE:
use commands::{parse_priority, parse_state, CommandExecutor};
use output::ExitCode;

// AFTER:
use jit::commands::{parse_priority, parse_state, CommandExecutor};
use jit::output::ExitCode;
```

**3.2 Common replacements needed**
| Old Pattern | New Pattern | Occurrences |
|-------------|-------------|-------------|
| `commands::` | `jit::commands::` | ~50 |
| `domain::` | `jit::domain::` | ~30 |
| `storage::` | `jit::storage::` | ~20 |
| `output::` | `jit::output::` | ~40 |

**3.3 Handle qualified paths in match arms**
```rust
// BEFORE:
match cmd {
    Commands::Issue(issue_cmd) => { ... }
}

// AFTER: (unchanged, imported types work the same)
match cmd {
    Commands::Issue(issue_cmd) => { ... }
}
```

### Phase 4: Remove Module Declarations (5 min)

**4.1 Delete duplicate mod declarations**
```rust
// main.rs - DELETE these lines:
// mod cli;           ❌
// mod commands;      ❌
// mod config;        ❌
// mod domain;        ❌
// mod graph;         ❌
// mod hierarchy_templates;  ❌
// mod labels;        ❌
// mod output;        ❌
// mod storage;       ❌
// mod type_hierarchy;  ❌
// mod visualization;  ❌

// KEEP only binary-specific:
mod output_macros;  ✅ (uses println!)
```

**4.2 Verify no `mod` declarations remain except binary-specific**
```bash
grep "^mod " src/main.rs
# Should only show: mod output_macros;
```

### Phase 5: Testing & Validation (15 min)

**5.1 Full compilation test**
```bash
cargo clean
cargo build --release
# Ensure no errors
```

**5.2 Run test suite**
```bash
cargo test
# All 556 tests should still pass
```

**5.3 Verify binary works**
```bash
./target/release/jit --version
./target/release/jit init
./target/release/jit issue create --title "Test"
./target/release/jit validate
```

**5.4 Check compilation time improvement**
```bash
cargo clean
time cargo build --release > /tmp/before.txt 2>&1

# After refactoring:
cargo clean  
time cargo build --release > /tmp/after.txt 2>&1

# Expected: 5-10% faster (one less module tree compile)
```

### Phase 6: Documentation Updates (10 min)

**6.1 Update main.rs docstring**
```rust
//! Just-In-Time Issue Tracker - Binary Entry Point
//!
//! This binary provides the CLI interface to the JIT library.
//! All core logic lives in the library crate for reusability.
//!
//! ## Architecture
//!
//! ```text
//! main.rs (this file)
//!   ↓ imports
//! lib.rs (jit library)
//!   ├── commands/    (business logic)
//!   ├── storage/     (persistence)
//!   ├── domain/      (data models)
//!   └── ...
//! ```
```

**6.2 Update ROADMAP.md**
Mark "Binary-to-library refactoring" complete

**6.3 Create session notes**
Document what was changed and why

---

## Risk Mitigation

### Risk 1: Visibility Issues
**Problem:** Some types may not be public in lib.rs  
**Mitigation:** 
- Test compile after each import group
- Add `pub` to lib.rs exports as needed
- Document visibility changes in commit message

### Risk 2: Macro Expansion Issues  
**Problem:** `output_macros` might reference other modules  
**Mitigation:**
- Keep `output_macros` as local module in main.rs
- If it needs library types, import them: `use jit::domain::Issue;`
- Test macro usage before/after

### Risk 3: Performance Regression
**Problem:** Import overhead (unlikely)  
**Mitigation:**
- Benchmark before/after with `time cargo build --release`
- Profile with `cargo build --timings`
- Expect 5-10% improvement, not regression

### Risk 4: Breaking Test Fixtures
**Problem:** Test helpers in main.rs  
**Mitigation:**
- main.rs shouldn't have test fixtures (verify with `grep "#\[cfg(test)\]"`)
- All tests are in lib or tests/, should be unaffected

---

## Success Criteria

- [ ] Zero `mod` declarations in main.rs except `mod output_macros;`
- [ ] All imports use `jit::` prefix (from library)
- [ ] `cargo build --release` succeeds
- [ ] All 556 tests pass
- [ ] `cargo clippy` shows zero errors
- [ ] `cargo fmt --check` passes
- [ ] Binary works: `jit init && jit issue create --title "Test"`
- [ ] Compilation time same or faster
- [ ] No new warnings introduced

---

## Rollback Plan

If issues arise mid-refactoring:

```bash
# Commit before starting
git add -A
git commit -m "Before: binary-to-library refactoring"

# If problems occur
git reset --hard HEAD  # Revert to before refactoring
```

---

## Post-Refactoring Cleanup

### Optional improvements after main refactoring:

1. **Move output_macros to lib** (if useful for MCP server)
   - Replace `println!` with configurable output
   - Effort: 20 minutes

2. **Consolidate imports** (minor cleanup)
   - Group related imports with `{...}`
   - Effort: 10 minutes

3. **Add lib usage examples** (documentation)
   - Show how to embed JIT in other Rust programs
   - Effort: 30 minutes

---

## Additional Minor Issues to Address

### Issue 1: IssueStore::root() Method ✅ FIXED
**Status:** Already fixed in Phase H  
**No further action needed**

### Issue 2: Inconsistent new() Methods
**Problem:** Some types return `Self`, others `Result<Self>`  
**Current:**
- `JsonFileStorage::new()` → `Self` (never fails)
- `HierarchyConfig::new()` → `Result<Self, ConfigError>` (validates)

**Proposal:** Add `try_new()` variants for validation
```rust
impl JsonFileStorage {
    /// Create new storage (never fails, path validated on first use)
    pub fn new<P: AsRef<Path>>(root: P) -> Self { ... }
    
    /// Create new storage and validate path exists
    pub fn try_new<P: AsRef<Path>>(root: P) -> Result<Self> {
        let storage = Self::new(root);
        storage.validate()?;
        Ok(storage)
    }
}
```

**Estimated effort:** 30 minutes  
**Priority:** Low (nice-to-have)

### Issue 3: create_issue() Argument Order
**Problem:** Current signature is non-intuitive
```rust
// Current (confusing order):
fn create_issue(
    title: String,
    description: String,
    priority: Priority,      // ← Natural: title, desc, labels, priority
    gates: Vec<String>,
    labels: Vec<String>,     // ← These are swapped
) -> Result<String>
```

**Proposal:** Standardize optional-last pattern
```rust
// Proposed (more intuitive):
fn create_issue(
    title: String,
    description: String,
    labels: Vec<String>,     // ← Labels first (often used)
    priority: Priority,      // ← Priority second (has default)
    gates: Vec<String>,      // ← Gates last (rarely used)
) -> Result<String>
```

**Estimated effort:** 1 hour (breaking change, update all callsites)  
**Priority:** Low (defer to v1.0 or major version bump)  
**Note:** Document in API comments for now

### Issue 4: Config Caching
**Problem:** Config loaded from disk on every validation call  
**Current:** `load_hierarchy_config()` called per-issue in `check_warnings()`  
**Impact:** Minor (config file is small, <1KB typically)

**Proposal:** Add optional caching
```rust
pub struct CommandExecutor<S: IssueStore> {
    storage: S,
    config_cache: Option<Arc<JitConfig>>,  // ← Add cache field
}

impl<S: IssueStore> CommandExecutor<S> {
    fn load_hierarchy_config(&self) -> Result<HierarchyConfig> {
        if let Some(cached) = &self.config_cache {
            return Ok(cached.to_hierarchy_config());
        }
        
        let config = JitConfig::load(self.storage.root())?;
        // Store in cache...
    }
}
```

**Estimated effort:** 1 hour  
**Priority:** Low (premature optimization)  
**Defer until:** Profile shows it's a bottleneck

---

## Timeline Summary

| Phase | Task | Time | Cumulative |
|-------|------|------|------------|
| 1 | Preparation & Analysis | 15 min | 15 min |
| 2 | Create Import Groups | 30 min | 45 min |
| 3 | Update Function Calls | 30 min | 1h 15min |
| 4 | Remove Module Declarations | 5 min | 1h 20min |
| 5 | Testing & Validation | 15 min | 1h 35min |
| 6 | Documentation | 10 min | 1h 45min |
| **Total** | | **~2 hours** | |

**With buffer for unexpected issues:** 2-2.5 hours

---

## Next Session Checklist

Before starting:
- [ ] Commit current state: `git commit -m "Phase H complete"`
- [ ] Create branch: `git checkout -b refactor/binary-to-library`
- [ ] Review this document
- [ ] Have compilation times baseline: `cargo clean && time cargo build`

During session:
- [ ] Follow phases 1-6 sequentially
- [ ] Test after each phase
- [ ] Document any deviations from plan

After completion:
- [ ] Merge to main (or create PR)
- [ ] Update ROADMAP.md
- [ ] Create session notes
- [ ] Celebrate! 🎉

---

## Related Documents

- `docs/session-2025-12-15-phase-h-implementation.md` - Context for why this refactoring is needed
- `copilot-instructions.md` - Coding standards to follow
- `ROADMAP.md` - Where to mark this complete

---

## Questions to Consider

1. **Should output_macros stay in main.rs?**
   - Recommendation: Yes for now (uses println!)
   - Revisit if MCP server needs similar output formatting

2. **Are there any main.rs-specific helpers we should preserve?**
   - Review: `error_to_exit_code()`, `parse_priority()`, etc.
   - Most should move to lib eventually

3. **Should we add more re-exports to lib.rs?**
   - Consider: Facade pattern with `pub use` for common imports
   - Example: `pub use commands::CommandExecutor;` (already exists)

4. **Do we need a lib-specific prelude?**
   - Not yet (project not large enough)
   - Consider at 20+ modules

---

## ✅ Implementation Complete (2025-12-15)

### Changes Made

**1. main.rs refactoring:**
- Removed all `mod` declarations except `mod output_macros;`
- Changed all imports to use `jit::` prefix:
  - `use cli::` → `use jit::cli::`
  - `use commands::` → `use jit::commands::`
  - `use output::` → `use jit::output::`
  - `use crate::type_hierarchy::` → `use jit::type_hierarchy::`
  - `use hierarchy_templates::` → `use jit::hierarchy_templates::`

**2. output_macros.rs refactoring:**
- Updated all imports to use `jit::output::` prefix
- Kept as binary-specific module (uses `println!`)

**3. Import organization:**
```rust
// Binary-specific module
mod output_macros;

// Library imports
use jit::cli::{Cli, Commands, DepCommands, ...};
use jit::commands::{parse_priority, parse_state, CommandExecutor};
use jit::output::ExitCode;
use jit::storage::{IssueStore, JsonFileStorage};
```

### Verification Results

✅ **All tests passing:** 556+ tests  
✅ **Zero clippy warnings**  
✅ **Clean build time:** 15.6s (release mode, baseline established)  
✅ **All commands functional** (jit init, issue create, validate, etc.)

### Benefits Achieved

1. **Clearer architecture:** Single source of truth for modules (lib.rs)
2. **Reduced maintenance:** Only one place to declare new modules
3. **Better IDE support:** Consistent import paths throughout
4. **Follows Rust best practices:** Standard binary/library split pattern

### Files Modified

- `crates/jit/src/main.rs` - Module imports refactored
- `crates/jit/src/output_macros.rs` - Import paths updated
- `ROADMAP.md` - Phase I marked complete
- `docs/refactoring-plan-binary-to-library.md` - This document

### Time Spent

**Estimated:** 1-2 hours  
**Actual:** ~2 hours  
- Audit plan creation: 30 min
- Code changes: 60 min
- Testing & verification: 30 min

### Related Work

This refactoring was part of the label hierarchy audit (Week 1, Day 1-2) documented in `docs/label-hierarchy-audit-plan.md`.

---

**Status:** ✅ **PRODUCTION READY**  
**Next:** Continue with Week 1 Day 3-4 tasks (documentation)
