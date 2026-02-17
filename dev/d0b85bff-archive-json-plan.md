# Add JSON Output Support to doc archive Command (d0b85bff)

## Problem

The `jit doc archive` command lacks `--json` flag, making it inconsistent with other commands and preventing proper structured warning output. Currently prints directly to stdout and warnings are dropped or printed to stderr as a workaround.

## Current State

**In `archive_document()` method:**
- 12+ `println!` calls scattered throughout (dry-run plan, success messages)
- Already returns `Vec<String>` warnings (done in f766b092)
- No structured data returned, only prints

**In main.rs:**
- Archive command lacks `json` field in CLI definition
- Warnings printed to stderr with eprintln! (TODO comment references this issue)
- No JSON output mode

## Implementation Plan

### 1. Add CLI Parameter (`cli.rs`)

```rust
Archive {
    path: String,
    category: String,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    force: bool,
    #[arg(long)]  // ADD THIS
    json: bool,
}
```

**Location:** `crates/jit/src/cli.rs` around line 807

### 2. Create Return Structure (`commands/document.rs`)

Add new struct to capture archive operation results:

```rust
#[derive(Debug, Serialize)]
pub struct ArchiveResult {
    pub source_path: String,
    pub dest_path: String,
    pub category: String,
    pub assets_moved: usize,
    pub updated_issues: Vec<String>,
    pub dry_run: bool,
}
```

**Location:** Near other document structs (DocumentListResult, DocumentHistory, etc.)

### 3. Refactor `archive_document()` Method

**Signature change:**
```rust
pub fn archive_document(
    &self,
    path: &str,
    category: &str,
    dry_run: bool,
    force: bool,
) -> Result<(ArchiveResult, Vec<String>)>  // Changed from Result<Vec<String>>
```

**Replace println! calls with data collection:**

Dry-run section (lines 1017-1038):
- Remove all `println!` calls
- Collect asset list for result
- Return early with ArchiveResult

Success section (lines 1074-1081):
- Remove `println!` calls
- Return ArchiveResult with:
  - source_path: `path.to_string()`
  - dest_path: `dest_path.to_str().unwrap().to_string()`
  - category: `category.to_string()`
  - assets_moved: `per_doc_assets.len()`
  - updated_issues: `updated_issues` (already collected)
  - dry_run: `false`

**Location:** `crates/jit/src/commands/document.rs` lines 963-1084

### 4. Update main.rs Handler

Update the archive command handler around line 2130:

```rust
DocCommands::Archive {
    path,
    category,
    dry_run,
    force,
    json,  // NOW AVAILABLE
} => {
    let output_ctx = OutputContext::new(quiet, json);
    let (result, warnings) = executor.archive_document(&path, &category, dry_run, force)?;
    
    // Print warnings
    for warning in warnings {
        output_ctx.print_warning(&warning)?;
    }
    
    if json {
        let output = JsonOutput::success(result, "doc archive");
        println!("{}", output.to_json_string()?);
    } else {
        if result.dry_run {
            println!("✓ Archival plan (--dry-run)\n");
            println!("  Document:");
            println!("    📄 {}", result.source_path);
            println!("       → {}", result.dest_path);
            println!("\n  Category: {}", result.category);
            
            if result.assets_moved > 0 {
                println!("\n  Assets to move: {}", result.assets_moved);
            } else {
                println!("\n  No per-doc assets found");
            }
            
            println!("\n  Run without --dry-run to execute.");
        } else {
            println!("✓ Archived successfully");
            println!("  {} → {}", result.source_path, result.dest_path);
            if result.assets_moved > 0 {
                println!("  Moved {} asset(s)", result.assets_moved);
            }
            if !result.updated_issues.is_empty() {
                println!("  Updated {} issue(s)", result.updated_issues.len());
            }
        }
    }
}
```

Remove the TODO(d0b85bff) comment.

### 5. Testing Strategy

**Manual Testing:**
```bash
# Test dry-run with JSON
jit doc archive dev/test.md --type old --dry-run --json

# Test actual archive with JSON
jit doc archive dev/test.md --type old --json

# Test without JSON (human-readable)
jit doc archive dev/test.md --type old

# Test with warnings (missing files, etc.)
```

**Automated Tests:**
- Existing document.rs tests should still pass
- Archive method already has test coverage, just verify JSON output works

### 6. Implementation Steps (TDD)

1. **Write test first** - Add test for archive with JSON output
2. **Add CLI parameter** - Update DocCommands::Archive in cli.rs
3. **Create ArchiveResult struct** - Add with Serialize derive
4. **Update method signature** - Change return type to include ArchiveResult
5. **Refactor dry-run path** - Collect data, return early
6. **Refactor success path** - Collect data, return result
7. **Update main.rs handler** - Add json parameter, format output
8. **Run tests** - Verify all tests pass
9. **Manual testing** - Test both modes
10. **Run clippy & fmt** - Zero warnings

## Pattern Reference

Follow the pattern from `add_document_reference()` which already supports JSON:
- Method returns `(data, warnings)` tuple
- main.rs uses `OutputContext` for warnings
- Both human and JSON modes supported
- Clean separation: commands/ returns data, main.rs formats

## Acceptance Criteria Checklist

- [ ] `json: bool` parameter added to `DocCommands::Archive`
- [ ] `ArchiveResult` struct created with all necessary fields
- [ ] `archive_document()` returns structured data instead of printing
- [ ] main.rs uses `OutputContext` for warnings
- [ ] Both `--json` and human-readable output work correctly
- [ ] Dry-run mode respects `--json` flag
- [ ] All existing tests pass
- [ ] Zero clippy warnings
- [ ] Code formatted with cargo fmt

## Notes

- This is a pure refactoring - no behavioral changes
- Archive already handles warnings correctly, just needs proper output
- Asset list details for dry-run can be simplified in JSON (just count)
- Main.rs can still do fancy formatting for human-readable mode
