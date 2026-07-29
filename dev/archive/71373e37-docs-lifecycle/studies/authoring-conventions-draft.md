# Document Authoring Conventions and Link Validation

**Status:** Draft - To be integrated into authoring conventions when created  
**Epic:** 71373e37-bb30-41d2-af0c-f08f381e027e (Documentation Lifecycle)  
**Date:** 2024-12-24  
**Author:** Implementation of jit doc check-links (fb6e2e31)

## Purpose

This document provides guidelines for writing documentation that is safe to archive and move without breaking links or losing assets. It should be integrated into the final authoring conventions document when the documentation lifecycle system is complete.

## Link Validation Overview

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

## Asset Management Best Practices

### Per-Document Assets (Recommended)

Store assets relative to their document for easy co-movement:

```
docs/
  design/
    authentication-design.md
    assets/
      auth-flow-diagram.png
      user-model-schema.png
```

**Markdown reference:**
```markdown
![Auth Flow](assets/auth-flow-diagram.png)
```

**Benefits:**
- Assets move with their document
- Easy to validate and archive together
- Clear ownership and organization

### Shared Assets (Use Sparingly)

Only use shared assets when truly needed by multiple documents:

```
docs/
  assets/
    shared/
      company-logo.png
```

**Warning:** Shared assets require careful coordination during archival.

## Link Safety Guidelines

### ✅ Safe Patterns

**1. Relative links within same directory:**
```markdown
See [implementation](implementation.md) for details.
```

**2. Single-level parent reference:**
```markdown
See [design doc](../design/auth-design.md).
```

**3. Root-relative links (use sparingly):**
```markdown
See [README](/README.md) at repository root.
```

### ⚠️ Risky Patterns

**Deep relative traversal (2+ parent levels):**
```markdown
![Asset](../../assets/shared/diagram.png)
```

**Why risky:** Moving the document breaks the link. The validator will warn about these.

**Better approach:** Move asset closer to document or use shared assets pattern intentionally.

### ❌ Avoid

**1. Absolute file system paths:**
```markdown
![Bad](/home/user/docs/image.png)  # NEVER DO THIS
```

**2. Relative links to files outside repository:**
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
![Diagram](assets/architecture.png)
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
| `risky_asset_path` | Deep relative path to asset | Consider moving asset closer |

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
        "document": "docs/design.md",
        "type": "risky_asset_path",
        "asset": "../../assets/diagram.png",
        "message": "Deep relative path '../../assets/diagram.png' may break if document is moved"
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

### Manual Archival

```bash
# 1. Validate before archival
jit doc check-links --scope issue:abc123

# 2. If exit code 0, proceed with archival
mv docs/design/feature-x.md .jit/docs/archive/features/

# 3. Co-move assets
mv docs/design/assets/ .jit/docs/archive/features/assets/
```

### Automated Archival (Future)

When `jit doc archive` is implemented, it will:
1. Run `check-links` automatically
2. Fail if errors found (exit code 1)
3. Warn if risky patterns detected (exit code 2)
4. Co-move assets with documents

## Examples

### Example 1: Clean Document

**File:** `docs/design/auth-design.md`
```markdown
# Authentication Design

![Auth Flow](assets/auth-flow.png)

See [implementation notes](../implementation/auth-impl.md).
```

**Validation:**
```bash
$ jit doc check-links --scope issue:auth-123
✅ All documents valid!
Summary: 1 document(s) checked, 0 error(s), 0 warning(s)
```

**Result:** Safe to archive

### Example 2: Document with Warnings

**File:** `docs/design/feature-x.md`
```markdown
# Feature X Design

![Old Diagram](../../old-assets/diagram.png)
![External](https://example.com/reference.png)
```

**Validation:**
```bash
$ jit doc check-links --scope issue:feature-x
⚠️  Warnings (2):
  docs/design/feature-x.md (risky_asset_path): Deep relative path '../../old-assets/diagram.png' may break if document is moved
  docs/design/feature-x.md (external_asset): External URL (not validated): https://example.com/reference.png

Summary: 1 document(s) checked, 0 error(s), 2 warning(s)
```

**Result:** Review warnings, consider fixing before archival

### Example 3: Document with Errors

**File:** `docs/design/broken.md`
```markdown
# Broken Document

![Missing](assets/gone.png)
See [nonexistent](missing.md).
```

**Validation:**
```bash
$ jit doc check-links --scope issue:broken
❌ Errors found (2):
  docs/design/broken.md (missing_asset): Asset not found: assets/gone.png
  docs/design/broken.md (broken_link): Document 'missing.md' not found (resolved to docs/design/missing.md)

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

Use issue-scoped validation when archiving specific features/epics.

## Recommendations for Authoring Conventions

When creating the final authoring conventions document, include:

1. **Mandatory validation** before archival operations
2. **CI/CD integration** to run check-links on PRs
3. **Pre-commit hooks** for local validation
4. **Asset organization standards** (per-doc vs shared)
5. **Link style guide** (when to use relative vs root-relative)
6. **External asset policy** (when acceptable, when to download)

## See Also

- Parent epic design: `dev/active/documentation-lifecycle-design.md`
- Implementation: Issue fb6e2e31 (jit doc check-links)
- Asset management: Issue 631bdd97 (jit doc assets commands)
