# Session Notes: Document Archive Implementation

**Issue:** #896ff7df - Implement jit doc archive with assets  
**Date:** 2025-12-27  
**Status:** 90% Complete - Ready for final polish  
**Next Session:** Complete remaining tests and polish

---

## What Was Accomplished This Session

### ✅ Phase 1: Asset Handling (COMPLETE)
- Implemented asset scanning using existing AssetScanner
- Asset classification: per-doc vs shared
- Support for both `assets/` and `<doc-name>_assets/` patterns
- Assets move atomically with document preserving directory structure
- **Tests:** 2 passing (basic + doc-name pattern)

### ✅ Phase 2: Link Validation (COMPLETE - CRITICAL)
- Pre-archival link integrity validation
- Detects relative links to shared assets (would break after move)
- Compares source vs destination directory depth
- Allows safe root-relative links
- Blocks dangerous relative shared links with actionable error messages
- **Tests:** 2 passing (root-relative success + relative failure)

### ✅ Phase 3: Atomic Operations (COMPLETE - CRITICAL)
- Production-safe temp directory pattern (`.jit/tmp/archive-{uuid}`)
- Unique temp filenames prevent collisions (`doc-{uuid}`, `asset-{idx}-{uuid}`)
- All-or-nothing guarantees with automatic rollback
- Proper error handling and cleanup
- **Tests:** 3 passing (atomic rollback + nested structure + name collisions)

### ✅ Code Quality
- All 7 integration tests passing
- Full test suite passing
- Clippy clean
- Properly formatted
- No code duplication
- Fixed critical bugs found during review (name collision, path handling)

---

## What Remains To Complete

### Priority 1: Missing Integration Tests (30-45 minutes)

**Required by acceptance criteria (5 tests missing):**

#### Test 1: Archive doc linked to active issue → failure (unless --force)
```rust
#[test]
fn test_archive_fails_when_linked_to_active_issue() {
    // 1. Create issue in state: in_progress
    // 2. Link document to issue
    // 3. Attempt archive WITHOUT --force
    // 4. Assert: fails with clear message about active issue
    // 5. Attempt archive WITH --force
    // 6. Assert: succeeds
}
```

#### Test 2: Archive permanent doc (docs/) → failure
```rust
#[test]
fn test_archive_fails_for_permanent_docs() {
    // 1. Create document in docs/ (permanent path)
    // 2. Attempt archive
    // 3. Assert: fails with clear message about permanent paths
}
```

#### Test 3: Dry-run doesn't mutate
```rust
#[test]
fn test_dry_run_no_mutation() {
    // 1. Create doc with assets
    // 2. Run archive with --dry-run
    // 3. Assert: output shows plan
    // 4. Assert: source files still exist
    // 5. Assert: destination files don't exist
    // 6. Assert: no event logged
}
```

#### Test 4: Issue metadata updated correctly
```rust
#[test]
fn test_issue_metadata_updates_after_archive() {
    // 1. Create issue with document reference
    // 2. Archive the document
    // 3. Load issue and verify DocumentReference.path updated to new location
    // 4. Verify issue can still load the document
}
```

#### Test 5: jit doc show still works after archival
```rust
#[test]
fn test_doc_show_works_post_archival() {
    // 1. Create issue with document
    // 2. Archive the document
    // 3. Run: jit doc show <issue-id>
    // 4. Assert: command succeeds and shows archived document path
}
```

**Acceptance:** All 12 integration tests pass (7 existing + 5 new)

---

### Priority 2: Post-Archival Verification (1 hour)

**Currently missing:** Step 11 of archival process

**What to implement:**
```rust
// After atomic move completes, before logging event
fn verify_post_archival_links(
    &self,
    dest_doc: &Path,
    repo_root: &Path,
) -> Result<()> {
    // 1. Read archived document content
    // 2. Scan for asset references
    // 3. Verify all per-doc assets exist at new location
    // 4. Verify relative paths still resolve
    // 5. Return error if any links broken (with rollback instructions)
}
```

**Add to archive_document() after atomic_archive_move:**
```rust
// Verify links still work post-move
self.verify_post_archival_links(&dest_path, repo_root)?;
```

**Acceptance:** Post-archival verification catches any broken links

---

### Priority 3: Polish (1-2 hours)

#### Enhanced Dry-Run Output
**Current:**
```
✓ Archival plan (--dry-run)

  Source: dev/active/feature-x-design.md
  Destination: dev/archive/features/feature-x-design.md
  Category: features

  Assets to move:
    ✓ dev/active/assets/diagram.png

  Run without --dry-run to execute.
```

**Improved:**
```
✓ Archival plan (--dry-run)

  Source: dev/active/feature-x-design.md
  Destination: dev/archive/features/feature-x-design.md
  Category: features

  Files to archive:
    📄 dev/active/feature-x-design.md
       → dev/archive/features/feature-x-design.md

  Assets to move (2):
    🖼️  dev/active/assets/diagram.png
       → dev/archive/features/assets/diagram.png
    🖼️  dev/active/assets/flow.svg
       → dev/archive/features/assets/flow.svg

  Shared assets (will not move):
    🔗 /docs/logo.png (root-relative link - safe)

  Issues to update (1):
    - #abc12345: Update document reference path

  Run without --dry-run to execute.
```

#### Better Error Messages

**Current link validation error:**
```
❌ Cannot archive: would break 1 relative link(s) to shared assets

Document: dev/active/feature-z-design.md
Destination: dev/archive/features/feature-z-design.md

Risky links:
  - ../../shared/common-diagram.png (relative link to shared asset)

Solutions:
1. Move asset to per-doc location (assets/ or <doc-name>_assets/)
2. Change to root-relative link (start with /)
3. Remove the links before archiving
```

**Improved:**
```
❌ Cannot archive: would break relative links

Document: dev/active/feature-z-design.md
Destination: dev/archive/features/feature-z-design.md
Depth change: 2 → 3 (links will break)

Problematic links (1):
  Line 5: ![Diagram](../../shared/common-diagram.png)
    → Shared asset with relative path
    → Would break after moving to different directory depth

Solutions:
1. Move asset to per-doc location:
   mkdir dev/active/assets
   mv shared/common-diagram.png dev/active/assets/
   Update link: ![Diagram](assets/common-diagram.png)

2. Change to root-relative link:
   ![Diagram](/shared/common-diagram.png)

3. Remove the link before archiving

Run 'jit doc check-links issue:<id>' for full link report.
```

#### Documentation Improvements

**Add to help text:**
```
Examples:
  # Archive a feature design
  jit doc archive dev/active/auth-design.md --type design

  # Preview what will be archived
  jit doc archive dev/active/bug-analysis.md --type analysis --dry-run

  # Force archive even if linked to active issue
  jit doc archive dev/sessions/session-42.md --type session --force

Notes:
  - Only archives development docs (dev/active, dev/studies, dev/sessions)
  - Permanent docs (docs/, dev/architecture/) cannot be archived
  - Per-doc assets are automatically moved with the document
  - Shared assets must use root-relative links (starting with /)
  - Document references in issues are automatically updated
```

**Add to CLI help struct in cli.rs:**
```rust
#[command(
    about = "Archive a document with its assets",
    long_about = "Archive a development document to the archive directory with its per-doc assets.

The document and its assets are moved atomically (all-or-nothing) to prevent partial state.
Issue metadata is automatically updated to reference the new location.

Only documents in managed paths (dev/active, dev/studies, dev/sessions) can be archived.
Permanent documentation (docs/, dev/architecture/) is protected from archival.

Link integrity is validated before archiving to prevent broken documentation.",
    after_help = "Examples:
  jit doc archive dev/active/feature-design.md --type design
  jit doc archive dev/sessions/session-42.md --type session --dry-run
  jit doc archive dev/active/old-design.md --type design --force"
)]
```

---

## Implementation Status Summary

### ✅ Complete (90%)
- Core functionality (100%)
- Atomic operations with rollback (100%)
- Link validation (100%)
- Asset handling with both patterns (100%)
- Configuration and CLI (100%)
- Basic integration tests (7/12)

### ⚠️ Remaining (10%)
- Integration tests (5 missing)
- Post-archival verification
- Polish (dry-run output, error messages, help text)

---

## Success Criteria Checklist

From issue #896ff7df acceptance criteria:

- ✅ `jit doc archive PATH --type CATEGORY` implemented
- ✅ `--dry-run` shows plan without executing
- ✅ `--force` overrides active issue check
- ✅ Pre-flight validation (path, category, link integrity)
- ✅ Asset classification (per-doc vs shared)
- ✅ Atomic move operation with rollback
- ✅ Metadata updates (issue DocumentReference paths)
- ✅ Event logging (document_archived)
- ⚠️ 12 integration tests passing (7/12 done)
- ⚠️ Help text with examples (basic done, needs polish)
- ✅ Consistent with other doc commands

---

## Next Session Task List

### Must Do (Complete The Feature)
1. [ ] Add 5 missing integration tests (30-45 min)
2. [ ] Implement post-archival link verification (1 hour)
3. [ ] Test full workflow end-to-end

### Should Do (Polish)
4. [ ] Enhanced dry-run output with file-by-file listing (30 min)
5. [ ] Improved error messages with line numbers and examples (30 min)
6. [ ] Expanded help text with examples (15 min)

### Final Steps
7. [ ] Run all tests
8. [ ] Pass all gates (tests, clippy, fmt)
9. [ ] Update issue state to done
10. [ ] Mark issue #896ff7df as complete

---

## Key Implementation Details For Next Session

### Where Things Are
- **Main implementation:** `crates/jit/src/commands/document.rs`
  - `archive_document()` - Main entry point (line ~1054)
  - `atomic_archive_move()` - Atomic operations (line ~1447)
  - `validate_archive_link_integrity()` - Link validation (line ~1384)
  - `scan_and_classify_assets()` - Asset scanning (line ~1291)

- **Tests:** `crates/jit/tests/doc_archive_tests.rs`
  - 7 tests currently implemented
  - TestRepo helper at top of file

- **CLI:** `crates/jit/src/cli.rs` and `crates/jit/src/main.rs`
  - Archive subcommand defined
  - Help text needs expansion

### Important Patterns To Follow
- **TDD:** Write failing test first, then implement
- **Functional style:** Use iterators, avoid mutations where possible
- **Error handling:** Use `Result` with descriptive errors, never panic in library code
- **Atomic operations:** All file operations through temp directory pattern
- **Test helpers:** Use `TestRepo` struct for all integration tests

### Commands For Testing
```bash
# Run archive tests
cargo test --test doc_archive_tests

# Run specific test
cargo test --test doc_archive_tests test_name

# Full test suite
cargo test --workspace --quiet

# Quality checks
cargo clippy --workspace --all-targets
cargo fmt --all

# Test the command manually (after jit init in a test repo)
cargo run -- doc archive dev/active/test.md --type design --dry-run
```

---

## Blockers / Issues

**None.** Implementation is clean and production-ready for core functionality.

---

## Notes

- **Critical bugs fixed:** Name collision in temp files, path handling inconsistencies
- **Regression tests added:** Ensure bugs don't reappear
- **Production-safe:** Atomic operations with rollback are solid
- **Link validation:** Works correctly, prevents broken documentation
- **Code quality:** All existing code is clean, tested, and well-documented

The foundation is excellent. Remaining work is straightforward testing and polish.
