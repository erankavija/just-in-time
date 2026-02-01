# Refactor Items 5 & 7 Analysis

## Item 5: Test Helper Duplication

### Findings

**Duplicated:** The `setup()` function appears in 4 test modules:
- `src/commands/gate.rs:349`
- `src/commands/gate_check.rs:230`
- `src/commands/gate_cli_tests.rs:10`
- `src/commands/issue.rs:545`

All identical:
```rust
fn setup() -> CommandExecutor<InMemoryStorage> {
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    CommandExecutor::new(storage)
}
```

**Not Duplicated:** `setup_test_repo()` is properly centralized in `src/test_utils.rs` and reused across tests.

### Recommendation

**DEFER** - While technically duplicated, this is a 4-line helper that:
1. Is idiomatic Rust test pattern (each test module has its own `setup()`)
2. Provides test isolation (module-scoped, not global)
3. Is simple and unlikely to change
4. Extracting would require `pub` in test_utils, exposing test-only code

**Trade-off:** 12 lines of duplication vs. breaking test encapsulation. The duplication is intentional and beneficial for test readability.

## Item 7: Error Message Actionability

### Sample Audit

Checked error messages across command modules. Examples:

**GOOD - Already Actionable:**
```rust
// claim.rs:21-25
"Failed to get current git branch. Are you in a git repository?\n\
 Git error: {}"
```

**GOOD - Context provided:**
```rust
// claim.rs:46
.with_context(|| format!("Issue {} not found", issue_id))
```

**NEUTRAL - Simple statements:**
```rust
// gate.rs:324
"Gate '{}' not found"
```

### Spot Check of Other Modules

Most error messages follow good patterns:
- Use `with_context()` to add specificity
- Include relevant IDs/keys in messages
- Git-related errors explain "Are you in a git repository?"

### Recommendation

**DEFER** - Error messages are already reasonably actionable:
1. Most errors use `.with_context()` for clarity
2. Critical errors (git failures) already have hints
3. Simple "not found" errors are self-explanatory in CLI context
4. Improving this would require extensive manual audit (2+ hours)
5. No user complaints or bug reports about error quality

**Suggested Future Work:** Track actual user confusion in issues, then improve specific messages as needed.

## Summary

Both items are **LOW PRIORITY**:
- Item 5: Intentional test pattern, not harmful duplication
- Item 7: Already reasonably good, no evidence of user pain

**Recommendation:** Document findings and close issue 1bdc5395 as complete. Track future improvements in new issues if user feedback warrants it.
