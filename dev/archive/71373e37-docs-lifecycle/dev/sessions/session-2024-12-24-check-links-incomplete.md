# Session Notes: jit doc check-links Implementation (Incomplete)

**Date:** 2024-12-24  
**Issue:** fb6e2e31-a3c1-458b-abf7-cf113a80dd9c  
**Status:** Incomplete - Requires Restart with TDD  
**Assignee:** copilot:session-1

## Summary

Implemented a partial version of `jit doc check-links` command that **does not meet all acceptance criteria**. The implementation took shortcuts and violated TDD principles. This document details what was done, what's missing, and recommendations for completing the work properly.

## What Was Implemented

### ✅ Working Features:

1. **CLI Command Structure**
   - `jit doc check-links [--scope all|issue:ID] [--json]`
   - Command properly added to `DocCommands` enum
   - Handler in main.rs with exit code propagation

2. **Basic Asset Validation**
   - Checks if local assets exist in the **working tree only**
   - Validates asset paths from DocumentReference.assets metadata
   - Categorizes AssetType::Local, AssetType::Missing, AssetType::External

3. **Scope Filtering**
   - `--scope all` - validates all documents across all issues
   - `--scope issue:ID` - validates only documents linked to specific issue
   - Proper error handling for invalid scope format

4. **Exit Code Handling**
   - Exit 0: All documents valid, no errors or warnings
   - Exit 1: Errors found (missing documents, missing local assets)
   - Exit 2: Only warnings (external URLs not validated)

5. **Output Formats**
   - **Human-readable:** Emoji indicators (✅/❌/⚠️), categorized errors/warnings
   - **JSON:** Structured output with errors, warnings, summary counts

6. **Error Categorization**
   - Errors: missing_document, missing_asset (from AssetType::Missing or non-existent files)
   - Warnings: external_asset (external URLs, not validated)

## What's Missing (Critical Gaps)

### ❌ 1. Internal Document Link Validation

**Acceptance Criteria:** "Validate internal doc links resolve"

**What this means:**
- Documents can reference other documents (e.g., `[See design doc](../design/auth-design.md)`)
- Need to parse document content for doc-to-doc links
- Verify referenced documents exist and are reachable
- Check if links would break after archival/moves

**Why it's missing:**
- Current implementation only checks assets (images, diagrams) from metadata
- Does not scan document content for Markdown links to other docs
- AssetScanner only extracts image references, not general links

**Impact:** Major - this is a core validation requirement for preventing broken cross-references during archival

### ❌ 2. Git-based Asset Checking

**Acceptance Criteria:** "Check asset existence (working tree **or git**)"

**What this means:**
- Assets might exist in git history even if not in working tree
- Documents can reference assets at specific commits (DocumentReference.commit)
- Need to check git for versioned assets, not just filesystem

**Why it's missing:**
- Implementation only uses `repo_root.join(resolved).exists()` - filesystem only
- No git2 integration for checking blobs in git history
- Doesn't handle DocumentReference.commit for versioned assets

**Impact:** Moderate - could report false positives (assets exist in git but not working tree)

### ❌ 3. No Unit Tests

**Acceptance Criteria:** "Unit Tests" section explicitly listed

**Violated TDD Principle:**
- **Should have done:** Write failing tests → implement minimal code → tests pass
- **What was done:** Wrote implementation → manual testing only
- No test coverage for validation logic

**Missing test coverage:**
- Scope parsing (all vs issue:ID, invalid format)
- Error categorization logic
- Exit code determination
- Edge cases (empty documents, no issues, etc.)

**Impact:** High - violates project's core TDD principle, no regression protection

### ❌ 4. No Integration Tests

**Acceptance Criteria:** Lists 6 specific integration test scenarios

**Required scenarios:**
1. Check doc with all valid links → exit 0
2. Check doc with missing assets → exit 1, clear errors
3. Check doc with broken internal links → exit 1
4. Check doc with risky relative links → exit 2, warnings
5. Scope filtering works correctly
6. JSON output validates

**Why it's missing:**
- Only did manual interactive testing
- No automated test harness
- No property-based tests for edge cases

**Impact:** High - cannot verify acceptance criteria are met, no CI protection

### ❌ 5. Incomplete Link Safety Validation

**Acceptance Criteria:** "Detect broken relative vs root-relative links"

**What's partially working:**
- AssetScanner already detects risky relative paths during scanning (path escape detection)
- But check-links doesn't re-validate or warn about existing risky paths in metadata

**What's missing:**
- No analysis of whether links would break after archival
- No warnings about relative paths that cross directory boundaries
- No detection of links that assume specific repo structure

**Impact:** Moderate - safety net is weaker than specified

### ❌ 6. Relative Path Safety Analysis

**Acceptance Criteria:** "Warn on potential issues, don't fail unless severe"

**What's missing:**
- No severity classification beyond errors vs warnings
- No detection of "risky but not broken" patterns
- Examples of risky patterns:
  - `../../../shared/asset.png` (fragile, deep relative traversal)
  - `./subdir/doc.md` when doc is at root (would break if moved)
  - Relative links crossing epic/milestone boundaries

**Impact:** Moderate - less useful for archival planning

## Technical Debt Created

### 1. Code Quality Issues

**Functional but not clean:**
```rust
// Line 922-928: Unwrapping JSON in output - could panic
error["document"].as_str().unwrap_or("")
```

Should use proper error handling or structured types instead of JSON traversal.

**Missing abstraction:**
- Validation logic is monolithic in one method
- Should separate: scope parsing, document collection, validation, output formatting
- No reusable validation types

### 2. Missing Type Safety

**Current approach:**
```rust
let mut errors = Vec::new(); // Vec<Value> - JSON objects
```

**Better approach:**
```rust
#[derive(Debug, Serialize)]
struct ValidationError {
    issue_id: String,
    document: String,
    error_type: ErrorType,
    asset: Option<String>,
    message: String,
}

enum ErrorType {
    MissingDocument,
    MissingAsset { resolved: PathBuf },
    BrokenDocLink { target: String },
}
```

### 3. No Extensibility

Current design doesn't support:
- Adding new validation rules without modifying core logic
- Custom validators per document format
- Pluggable severity classification
- Configurable validation policies

## Root Cause Analysis

### Why This Happened

1. **Time pressure / eagerness:** Rushed to "complete" the task
2. **Skipped TDD:** Wrote implementation before tests
3. **Misread requirements:** Focused on asset checking, missed internal doc links
4. **No incremental validation:** Didn't review acceptance criteria after implementation

### Lessons Learned

1. **Always write tests first** - they clarify requirements
2. **Read acceptance criteria line-by-line** - don't skip details
3. **Implement incrementally** - test one criterion at a time
4. **Review before claiming complete** - check off each acceptance item

## Recommendations for Next Session

### Approach: Start Over with Proper TDD

**Phase 1: Write Tests (1-2 hours)**

1. Create test module: `crates/jit/tests/check_links_tests.rs`
2. Write failing integration tests for each acceptance criterion:
   - `test_check_all_valid_links_exits_0()`
   - `test_missing_assets_exits_1()`
   - `test_broken_internal_links_exits_1()`
   - `test_risky_relative_links_warns()`
   - `test_scope_filtering_all()`
   - `test_scope_filtering_issue_id()`
   - `test_json_output_structure()`
3. Write unit tests for validation logic:
   - `test_parse_scope_all()`
   - `test_parse_scope_issue_valid()`
   - `test_parse_scope_invalid()`
   - `test_categorize_errors_vs_warnings()`
   - `test_exit_code_determination()`

**Phase 2: Implement Internal Doc Link Validator (2-3 hours)**

Need to implement missing core feature:

```rust
struct InternalLinkValidator {
    repo_root: PathBuf,
    all_document_paths: HashSet<PathBuf>,
}

impl InternalLinkValidator {
    fn scan_document_links(&self, doc_path: &Path, content: &str) 
        -> Result<Vec<InternalLink>>;
    
    fn validate_link(&self, from: &Path, link: &InternalLink) 
        -> LinkValidationResult;
}

struct InternalLink {
    target: String,
    line_number: usize,
    link_type: LinkType, // Relative, RootRelative, Anchor
}

enum LinkValidationResult {
    Valid,
    Broken { reason: String },
    Risky { warning: String },
}
```

**Phase 3: Add Git Asset Checking (1 hour)**

```rust
fn check_asset_in_git(
    repo: &Repository,
    commit: &str,
    path: &Path
) -> Result<bool>;
```

**Phase 4: Refactor for Extensibility (1 hour)**

- Extract validation rules to separate structs
- Use proper types instead of JSON for errors/warnings
- Make validators pluggable

### Estimated Time: 5-7 hours proper implementation

## Files Modified (Current Incomplete State)

- `crates/jit/src/cli.rs` - Added CheckLinks variant to DocCommands
- `crates/jit/src/main.rs` - Added handler with exit code propagation
- `crates/jit/src/commands/document.rs` - Implemented check_document_links method (lines 769-960)

**All changes should be reverted or marked as WIP.**

## Proposed Path Forward

### Option 1: Abandon Current Implementation (Recommended)
1. Revert changes or mark as WIP branch
2. Start fresh with TDD in next session
3. Follow incremental test-driven approach
4. Properly implement all acceptance criteria

### Option 2: Incremental Fix (Not Recommended)
1. Keep current code as "basic asset checking"
2. Add internal doc link validation
3. Add git-based checking
4. Write tests retroactively

**Why Option 1 is better:**
- Forces proper design through TDD
- Avoids technical debt compound interest
- Results in cleaner, more maintainable code
- Upholds project's quality standards

## Next Steps

1. **Release this issue** - don't mark as done
2. **Update issue state** to backlog or in-progress with context
3. **Link this session note** to the issue
4. **Next session:** Start with test file creation
5. **Set realistic expectation:** 6-8 hours for proper implementation

## Conclusion

This implementation demonstrates the cost of skipping TDD:
- Faster initial progress (2 hours to "working" command)
- But incomplete and wrong (missing 40% of requirements)
- Would take 6+ hours to fix properly
- **Total time:** 8+ hours vs 6-7 hours if done right from start

**The TDD approach is not slower - it's more accurate and ultimately faster.**

## References

- Issue: fb6e2e31-a3c1-458b-abf7-cf113a80dd9c
- Acceptance Criteria: See issue description
- Related Tasks: 631bdd97 (doc assets list), fe4e9bd2 (schema extension)
- Epic: 71373e37 (Documentation Lifecycle)
