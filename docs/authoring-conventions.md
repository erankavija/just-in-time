# Development Documentation Authoring Conventions

## Overview

This guide provides conventions for writing **development documentation** that is safe to archive and move without breaking links or losing assets. These conventions apply to lifecycle-managed documentation in the `dev/` directory.

**Important distinction:**
- **User-facing docs** (`docs/`) - Permanent product documentation, not lifecycle-managed
- **Development docs** (`dev/`) - Contributor documentation with lifecycle management via jit

This document focuses on authoring **development documentation** that will be managed by jit's document lifecycle system.

## Link Validation

The `jit doc check-links` command validates documents before archival to ensure:
- Assets (images, diagrams) exist and are accessible
- Internal document links resolve correctly
- Relative paths won't break when documents are moved

### Command Usage

```bash
# Check all documents
jit doc check-links --scope all

# Check documents for specific issue
jit doc check-links --scope issue:abc123

# Machine-readable output for automation
jit doc check-links --json
```

### Exit Codes

- **0** - All documents valid, safe to proceed with archival
- **1** - Errors found (missing assets, broken links) - **DO NOT ARCHIVE**
- **2** - Warnings only (external URLs, risky paths) - Review before archiving

## Asset Management Patterns

### Pattern 1: Per-Document Assets (Recommended)

Store assets in a directory named after the document for easy co-movement:

```
dev/
  active/
    authentication-design.md
    authentication-design/
      auth-flow-diagram.png
      user-model-schema.png
```

**Markdown reference:**
```markdown
![Auth Flow](authentication-design/auth-flow-diagram.png)
```

**Benefits:**
- Assets move with their document during archival
- Easy to validate and archive together
- Clear ownership and organization
- Relative links remain valid after moving

**On archival:**
```bash
# Both document and assets move together
mv dev/active/authentication-design.md dev/archive/features/
mv dev/active/authentication-design/ dev/archive/features/
# Links still work - relative paths preserved
```

### Pattern 2: Shared Assets (Use Sparingly)

Only use shared assets when truly needed by multiple documents:

```
dev/
  diagrams/
    system-architecture.png
  active/
    feature-a-design.md
    feature-b-design.md
```

**Link with root-relative paths:**
```markdown
![System Architecture](/dev/diagrams/system-architecture.png)
```

**Warning:** Shared assets require careful coordination during archival. Use for:
- Architecture diagrams referenced by multiple features
- Common workflow diagrams
- Reusable technical illustrations

### Pattern 3: External Assets

For external resources:

```markdown
![Rust Book](https://doc.rust-lang.org/book/cover.png)
[GitHub Issue](https://github.com/org/repo/issues/123)
```

**Note:** External URLs are preserved but not bundled in snapshots (marked as external references).

## Link Safety Guidelines

### ✅ Safe Patterns

**1. Relative links within same directory:**
```markdown
See [implementation](implementation.md) for details.
```

**2. Per-doc assets (named directory pattern):**
```markdown
![Diagram](my-design/architecture.png)
```

**3. Single-level parent reference:**
```markdown
See [design doc](../active/auth-design.md).
```

**4. Root-relative links (for shared assets):**
```markdown
See [architecture](/dev/architecture/core-system-design.md) for context.
![Shared Diagram](/dev/diagrams/system-overview.png)
```

### ⚠️ Risky Patterns

**Deep relative traversal (2+ parent levels):**
```markdown
![Asset](../../diagrams/shared/diagram.png)
```

**Why risky:** Moving the document breaks the link. The validator will warn about these.

**Better approach:** 
- Use per-doc assets pattern instead, OR
- Use root-relative paths for intentionally shared assets

### ❌ Avoid

**1. Absolute file system paths:**
```markdown
![Bad](/home/user/docs/image.png)  # NEVER DO THIS
```

**2. Relative links escaping repository:**
```markdown
[External](../../../other-repo/doc.md)  # AVOID
```

## Asset Types and Validation

### Local Assets

Images, diagrams, PDFs stored in the repository.

**Validation:**
- Checked in working tree first
- Falls back to git history if not in working tree
- Warns about deep relative paths (2+ `../`)

**Example:**
```markdown
![Architecture Diagram](feature-x-design/architecture.png)
```

### External Assets

URLs to external resources.

**Validation:**
- Not validated (external availability not guaranteed)
- Generates warnings for tracking
- Consider downloading and storing locally for critical assets

**Example:**
```markdown
![External Diagram](https://example.com/diagram.png)  # WARNING
```

## Internal Document Links

Links between documents are validated to ensure the target exists.

### ✅ Valid
```markdown
See [authentication design](authentication-design.md).
```

### ❌ Invalid
```markdown
See [nonexistent doc](missing.md).  # ERROR: Document not found
```

## Pre-Archival Checklist

Before archiving a document:

1. **Run validation:**
   ```bash
   jit doc check-links --scope issue:<issue-id>
   ```

2. **Exit code 0:** Safe to archive
3. **Exit code 1:** Fix errors before archiving
4. **Exit code 2:** Review warnings, decide if acceptable

## Validation Error Types

### Errors (Exit code 1)

| Type | Meaning | Action |
|------|---------|--------|
| `missing_document` | Referenced document doesn't exist | Check path or create document |
| `missing_asset` | Asset not found in working tree or git | Add asset or fix path |
| `broken_link` | Internal doc link to nonexistent file | Fix link or create target |

### Warnings (Exit code 2)

| Type | Meaning | Action |
|------|---------|--------|
| `external_asset` | External URL not validated | Consider downloading locally |
| `risky_link` | Deep relative path or untracked doc | Review if intentional |
| `risky_asset_path` | Deep relative path to asset | Consider per-doc pattern |

## JSON Output Format

For automation and scripting:

```json
{
  "success": true,
  "data": {
    "valid": true,
    "errors": [],
    "warnings": [
      {
        "issue_id": "abc123...",
        "document": "dev/active/design.md",
        "type": "risky_asset_path",
        "asset": "../../diagrams/diagram.png",
        "message": "Deep relative path '../../diagrams/diagram.png' may break if document is moved"
      }
    ],
    "summary": {
      "total_documents": 1,
      "valid": 1,
      "errors": 0,
      "warnings": 1
    }
  }
}
```

## Integration with Archival Workflow

### Automated Archival

The `jit doc archive` command integrates validation:

```bash
# Validate and archive in one operation
jit doc archive dev/active/feature-x.md --type features

# Preview without executing
jit doc archive dev/active/feature-x.md --type features --dry-run

# Force archival despite warnings
jit doc archive dev/active/feature-x.md --type features --force
```

**Archival process:**
1. Validates links automatically (`check-links`)
2. Fails if errors found (exit code 1)
3. Warns if risky patterns detected (exit code 2)
4. Co-moves per-doc assets with documents
5. Updates issue metadata with new paths
6. Preserves shared assets in original location

### Archive Categories

Categories are configured in `.jit/config.toml`:

```toml
[documentation.categories]
features = "Feature designs and implementation plans"
bug-fixes = "Bug analyses and resolutions"
refactorings = "Code improvement documentation"
studies = "Research and investigations"
sessions = "Development session notes"
```

Categories are domain-agnostic and can be customized per project.

## Examples

### Example 1: Clean Document

**File:** `dev/active/auth-design.md`
```markdown
# Authentication Design

![Auth Flow](auth-design/auth-flow.png)

See [implementation notes](../studies/auth-impl-strategy.md).
```

**Validation:**
```bash
$ jit doc check-links --scope issue:auth-123
✅ All documents valid!
Summary: 1 document(s) checked, 0 error(s), 0 warning(s)
```

**Result:** Safe to archive

### Example 2: Document with Warnings

**File:** `dev/active/feature-x.md`
```markdown
# Feature X Design

![Old Diagram](../../old-diagrams/diagram.png)
![External](https://example.com/reference.png)
```

**Validation:**
```bash
$ jit doc check-links --scope issue:feature-x
⚠️  Warnings (2):
  dev/active/feature-x.md (risky_asset_path): Deep relative path '../../old-diagrams/diagram.png' may break if document is moved
  dev/active/feature-x.md (external_asset): External URL (not validated): https://example.com/reference.png

Summary: 1 document(s) checked, 0 error(s), 2 warning(s)
```

**Result:** Review warnings, consider fixing before archival

### Example 3: Document with Errors

**File:** `dev/active/broken.md`
```markdown
# Broken Document

![Missing](broken/gone.png)
See [nonexistent](missing.md).
```

**Validation:**
```bash
$ jit doc check-links --scope issue:broken
❌ Errors found (2):
  dev/active/broken.md (missing_asset): Asset not found: broken/gone.png
  dev/active/broken.md (broken_link): Document 'missing.md' not found (resolved to dev/active/missing.md)

Summary: 1 document(s) checked, 2 error(s), 0 warning(s)
```

**Result:** DO NOT ARCHIVE - fix errors first

## Technical Notes

### Link Detection

- **Markdown links:** `[text](url)` - Validated
- **Image links:** `![alt](url)` - Validated as assets
- **Anchor links:** `[text](#anchor)` - Skipped (same-document)
- **External URLs:** `http://` or `https://` - Warning only

### Git-based Asset Checking

Assets are checked in:
1. **Working tree** first (current files)
2. **Git HEAD** as fallback (committed files)

This allows validation even if assets were committed but not in working tree.

### Scope Filtering

- `--scope all`: Validates all documents across all issues
- `--scope issue:ID`: Validates only documents linked to specific issue

Use issue-scoped validation when archiving specific features or epics.

## Recommended Workflow

### During Development

1. **Follow per-doc assets pattern** for new documents
2. **Name asset directory after document** (e.g., `my-design.md` → `my-design/`)
3. **Use relative links** for per-doc assets
4. **Use root-relative links** for intentionally shared assets
5. **Avoid deep relative traversal** (2+ `../`)

### Before Archival

1. **Validate links:** `jit doc check-links --scope issue:<id>`
2. **Fix errors** if exit code 1
3. **Review warnings** if exit code 2
4. **Archive with confidence:** `jit doc archive <path> --type <category>`

### CI/CD Integration

Add validation to pull request checks:

```bash
# In CI pipeline
jit doc check-links --scope all --json > validation-results.json

# Fail on errors
if [ $? -eq 1 ]; then
  echo "❌ Document validation failed"
  exit 1
fi
```

## See Also

- [Development Documentation Index](../dev/index.md) - Organization and lifecycle
- [Documentation Lifecycle Design](../dev/active/documentation-lifecycle-design.md) - System design
- [README.md](README.md) - Documentation navigation and overview
