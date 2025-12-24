# Implementation Guide: jit doc archive with assets

**Issue:** 896ff7df-e5b0-48b6-a3b0-a5745ff683b7  
**Task:** Implement jit doc archive with assets  
**Status:** Ready to start (dependency check-links completed)  
**Created:** 2024-12-24  

## Overview

Implement safe document archival that moves development documents with their per-doc assets while preserving link integrity. This is the **core payoff** for the asset management system - enabling safe archival without breaking anything.

## Updated Directory Structure (Phase 0 Redesign)

**IMPORTANT:** The archive structure was redesigned in Phase 0. Do NOT use `.jit/docs/archive/`.

### Current Structure

```
dev/                        # Development documentation (lifecycle-managed)
├── active/                 # Work-in-progress (linked to active issues)
├── architecture/           # Permanent reference (never archived)
├── vision/                 # Future plans (may archive if abandoned)
├── studies/                # Investigations (archive when complete)
├── sessions/               # Session notes (archive after 1-2 releases)
└── archive/                # ARCHIVE DESTINATION
    ├── features/           # From dev/active/ (feature designs)
    ├── bug-fixes/          # From dev/active/ (bug analyses)
    ├── refactorings/       # From dev/active/ (refactoring plans)
    ├── studies/            # From dev/studies/ (completed investigations)
    └── sessions/           # From dev/sessions/ (old session notes)

docs/                       # Product documentation (PERMANENT - never archived)
├── concepts/
├── tutorials/
├── how-to/
├── reference/
└── case-studies/
```

### Configuration

Set in `.jit/config.toml`:

```toml
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]  # Never archived

# Archive categories (configurable per project)
categories.design   = "features"
categories.analysis = "bug-fixes"
categories.refactor = "refactorings"
categories.session  = "sessions"
categories.study    = "studies"
```

## Domain-Agnostic Design

While JIT itself uses software-specific categories, the system is **domain-agnostic**. Projects can configure categories to match their domain:

### Software Development (current JIT config)
```toml
categories.design   = "features"
categories.analysis = "bug-fixes"
categories.refactor = "refactorings"
```

### Research Project
```toml
categories.experiment = "experiments"
categories.analysis   = "analyses"
categories.literature = "literature-reviews"
categories.study      = "pilot-studies"
```

### Knowledge Work
```toml
categories.project = "completed-projects"
categories.review  = "quarterly-reviews"
categories.study   = "research"
```

## Command Specification

### Command Syntax

```bash
jit doc archive PATH [--type CATEGORY] [--dry-run] [--force]
```

### Parameters

- **PATH** (required): Document to archive (repo-relative path)
  - Example: `dev/active/feature-x-design.md`
  
- **--type CATEGORY** (required): Archive category from config
  - Must match a configured category in `[documentation.categories]`
  - Example: `--type features` or `--type studies`
  - Error if category not configured
  
- **--dry-run** (flag): Show plan without executing
  - Display source → destination mapping
  - Show which assets will move
  - Show metadata updates
  - Exit without making changes
  
- **--force** (flag): Override safety checks
  - Allow archival even if linked to active issue
  - Use with caution

### Exit Codes

- **0**: Success (archived successfully)
- **1**: Errors (validation failed, safety check failed)
- **2**: Warnings (use --force to override)

## Archival Logic

### Pre-flight Validation

Before archiving, validate:

1. **Document exists** and is in a managed path (`dev/active`, `dev/studies`, `dev/sessions`)
2. **Category is configured** in `.jit/config.toml`
3. **Not a permanent doc** (refuse if in `permanent_paths`)
4. **Link integrity** (run check-links validation)
5. **Issue state** (warn if linked to active issue, block unless `--force`)

### Asset Classification

Assets are classified during archival:

**Per-doc assets (MOVE with document):**
- Located in `<doc-dir>/assets/` or `<doc-dir>/<doc-name>_assets/`
- Relative links from document
- Example: `dev/active/feature-x-design.md` → `dev/active/assets/diagram.png`

**Shared assets (STAY in place):**
- Located outside per-doc asset directories
- Root-relative links: `/docs/diagrams/shared.png` → **OK**
- Relative links: `../../shared/diagram.png` → **FAIL** (would break)

**External assets (no action):**
- URLs: `https://example.com/image.png`
- Tracked but not moved

### Validation Rules

| Asset Type | Link Type | Action | Status |
|------------|-----------|--------|--------|
| Per-doc | Relative | Move with doc | ✅ OK |
| Shared | Root-relative | Stay in place | ✅ OK |
| Shared | Relative | - | ❌ FAIL |
| External | Any | Note in metadata | ⚠️ WARN |
| Linked to active issue | Any | - | ⚠️ WARN (--force to override) |

### Destination Path Computation

```
Source: dev/active/feature-x-design.md
Category: features
Destination: dev/archive/features/feature-x-design.md

Source: dev/active/assets/diagram.png
Destination: dev/archive/features/assets/diagram.png
```

**Path algorithm:**
1. Strip managed path prefix: `dev/active/feature-x-design.md` → `feature-x-design.md`
2. Prepend archive destination: `dev/archive/features/feature-x-design.md`
3. Preserve directory structure within category

## Implementation Steps

### 1. Command Handler Setup

Add to `src/cli.rs`:

```rust
/// Archive a document with its assets
Archive {
    /// Document path to archive
    path: String,
    
    /// Archive category (must be configured in config.toml)
    #[arg(long)]
    r#type: String,
    
    /// Show plan without executing
    #[arg(long)]
    dry_run: bool,
    
    /// Override safety checks
    #[arg(long)]
    force: bool,
}
```

### 2. Core Archival Function

```rust
impl<S: IssueStore> CommandExecutor<S> {
    pub fn archive_document(
        &self,
        path: &str,
        category: &str,
        dry_run: bool,
        force: bool,
    ) -> Result<()> {
        // 1. Load configuration
        // 2. Validate document is archivable
        // 3. Scan document and classify assets
        // 4. Run link integrity check
        // 5. Compute destination paths
        // 6. Check for active issue links (fail unless --force)
        // 7. Show plan if dry_run
        // 8. Execute atomic move
        // 9. Update issue metadata
        // 10. Verify links still work
        // 11. Log event
    }
}
```

### 3. Validation Logic

```rust
fn validate_archivable(
    path: &Path,
    config: &DocConfig,
) -> Result<()> {
    // Check path is in managed_paths
    // Check path is not in permanent_paths
    // Check file exists
    // Check not already in archive
}

fn validate_category(
    category: &str,
    config: &DocConfig,
) -> Result<PathBuf> {
    // Check category exists in config.categories
    // Return archive destination path
}

fn check_active_issue_links(
    path: &str,
    storage: &Storage,
    force: bool,
) -> Result<Vec<String>> {
    // Find issues linking to this document
    // Filter to active/in-progress issues
    // Return list or fail if any found and !force
}
```

### 4. Atomic Move Operation

```rust
fn atomic_archive_move(
    source_doc: &Path,
    dest_doc: &Path,
    assets: &[AssetMove],
    repo_root: &Path,
) -> Result<()> {
    // Create temp directory
    let temp = TempDir::new()?;
    
    // Copy all files to temp
    // Verify all copies successful
    
    // Create destination directories
    // Move from temp to final destination (atomic renames)
    
    // Verify all moves successful
    // On error: rollback (delete destinations, restore from temp)
    
    // Delete sources only after all destinations verified
}
```

### 5. Metadata Update

```rust
fn update_issue_metadata(
    old_path: &str,
    new_path: &str,
    storage: &Storage,
) -> Result<Vec<String>> {
    // Find all issues referencing old_path
    // Update DocumentReference.path to new_path
    // Save updated issues
    // Return list of updated issue IDs
}
```

### 6. Event Logging

```rust
// Log document_archived event
{
    "event_type": "document_archived",
    "timestamp": "2024-12-24T20:00:00Z",
    "document": {
        "source": "dev/active/feature-x-design.md",
        "destination": "dev/archive/features/feature-x-design.md",
        "category": "features"
    },
    "assets_moved": 3,
    "issues_updated": ["abc123...", "def456..."]
}
```

## Integration with Existing Systems

### Use check-links for validation

```rust
// Before archiving, validate link integrity
let check_result = self.check_document_links(
    &format!("issue:{}", issue_id),
    false, // not JSON
)?;

if check_result != 0 {
    return Err(anyhow!("Link validation failed. Fix errors before archiving."));
}
```

### Use existing asset scanning

```rust
use crate::document::{AdapterRegistry, AssetScanner};

let registry = AdapterRegistry::with_builtins();
let adapter = registry.resolve(path, &content)?;
let scanner = AssetScanner::new(repo_root);
let assets = scanner.scan_assets(path, &content, adapter.as_ref())?;
```

## Test Strategy

### Unit Tests

```rust
#[test]
fn test_validate_archivable_managed_path() { }

#[test]
fn test_validate_archivable_permanent_path_fails() { }

#[test]
fn test_validate_category_exists() { }

#[test]
fn test_validate_category_not_configured_fails() { }

#[test]
fn test_compute_destination_path() { }

#[test]
fn test_classify_assets_per_doc() { }

#[test]
fn test_classify_assets_shared() { }
```

### Integration Tests

Required test scenarios from acceptance criteria:

```rust
#[test]
fn test_archive_doc_with_per_doc_assets() {
    // Create doc with assets/ subdirectory
    // Archive successfully
    // Verify doc and assets moved
    // Verify relative links still work
}

#[test]
fn test_archive_doc_with_root_relative_shared_assets() {
    // Create doc with root-relative links to shared assets
    // Archive successfully
    // Verify doc moved, shared assets stayed
    // Verify root-relative links still work
}

#[test]
fn test_archive_doc_with_relative_shared_links_fails() {
    // Create doc with relative links to shared assets
    // Attempt archive
    // Should fail with clear error
}

#[test]
fn test_archive_linked_to_active_issue_fails() {
    // Create active issue with linked doc
    // Attempt archive without --force
    // Should fail with warning about active issue
}

#[test]
fn test_archive_with_force_overrides_active_check() {
    // Create active issue with linked doc
    // Archive with --force
    // Should succeed
}

#[test]
fn test_dry_run_shows_plan_no_mutation() {
    // Create doc with assets
    // Run archive with --dry-run
    // Verify plan shown
    // Verify no files moved
}

#[test]
fn test_atomic_rollback_on_error() {
    // Simulate error during move
    // Verify rollback leaves no partial state
    // Verify source files still intact
}

#[test]
fn test_metadata_update_preserves_references() {
    // Create issue with doc reference
    // Archive doc
    // Verify issue still references doc at new path
    // Verify jit doc show still works
}
```

### Property-Based Tests (Optional)

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_path_normalization_no_escape(
        category in "[a-z]{3,10}",
        filename in "[a-z0-9-]{3,20}\\.md"
    ) {
        let dest = compute_destination(&filename, &category);
        assert!(dest.starts_with("dev/archive/"));
        assert!(!dest.contains(".."));
    }
}
```

## Error Messages

Provide helpful, actionable error messages:

```
❌ Cannot archive: document is in permanent path
  
  Document: docs/tutorials/quickstart.md
  
  Product documentation in docs/ is permanent and cannot be archived.
  Only development documentation in dev/ can be archived.

❌ Cannot archive: would break shared asset links

  Document: dev/active/design.md
  Problem: Relative link to shared asset would break
  
  Link: ../../shared/diagram.png
  Type: Relative path to shared asset
  
  Solutions:
  1. Move asset to per-doc location: dev/active/assets/diagram.png
  2. Change to root-relative link: /shared/diagram.png
  3. Accept broken link and fix manually after archival

⚠️  Warning: document linked to active issues

  Document: dev/active/feature-x.md
  Linked to: 2 active issues
    - abc123: Implement feature X (state: in-progress)
    - def456: Design review for X (state: ready)
  
  These issues still reference this document.
  Archive anyway? Use --force to proceed.

✓ Archival plan (--dry-run)

  Source: dev/active/authentication-design.md
  Destination: dev/archive/features/authentication-design.md
  Category: features
  
  Assets to move:
    ✓ dev/active/assets/auth-flow.png → dev/archive/features/assets/auth-flow.png
    ✓ dev/active/assets/user-model.png → dev/archive/features/assets/user-model.png
  
  Shared assets (will not move):
    - /docs/diagrams/company-logo.png (root-relative, safe)
  
  Issues to update:
    - Issue abc123: Update path reference
  
  Run without --dry-run to execute.
```

## Success Criteria Checklist

From issue 896ff7df acceptance criteria:

- [ ] `jit doc archive PATH --type CATEGORY` implemented
- [ ] `--dry-run` shows plan without executing
- [ ] `--force` overrides active issue check
- [ ] Pre-flight validation (path, category, link integrity)
- [ ] Asset classification (per-doc vs shared)
- [ ] Atomic move operation with rollback
- [ ] Metadata updates (issue DocumentReference paths)
- [ ] Event logging (document_archived)
- [ ] 6 integration tests passing
- [ ] Help text with examples
- [ ] Consistent with other doc commands

## Related Documentation

- **Design:** `dev/active/documentation-lifecycle-design.md` (epic 71373e37)
- **Organization:** `dev/studies/documentation-organization-strategy.md` (issue 165cf162)
- **Authoring:** `dev/active/authoring-conventions-draft.md` (from check-links task)
- **Dependencies:** Issue fb6e2e31 (check-links) - ✅ DONE

## Configuration Reference

Minimal `.jit/config.toml` for archival:

```toml
[documentation]
development_root = "dev"
managed_paths = ["dev/active", "dev/studies", "dev/sessions"]
archive_root = "dev/archive"
permanent_paths = ["docs/"]

# Categories must be explicitly configured
categories.design   = "features"
categories.analysis = "bug-fixes"
categories.refactor = "refactorings"
categories.session  = "sessions"
categories.study    = "studies"

# Future expansion (Phase 2)
# block_if_linked_to_active_issue = true
# retention_releases = 2
```

## Implementation Estimate

**Complexity:** Medium-High  
**Estimated Time:** 6-8 hours

**Breakdown:**
- Configuration loading and validation: 1 hour
- Path computation and validation: 1 hour
- Asset classification integration: 1 hour
- Atomic move with rollback: 2 hours
- Metadata updates: 1 hour
- Integration tests: 2 hours
- Documentation and help text: 30 minutes

## Notes for Implementation

1. **Read config first** - Load `.jit/config.toml` to get category mappings
2. **Reuse check-links** - Don't reimplement validation, call existing code
3. **Atomic operations** - Use temp directory + rename pattern for safety
4. **Test rollback** - Explicitly test error conditions and rollback
5. **Update session notes** - Track any deviations or design decisions
