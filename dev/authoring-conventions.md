# Development Documentation Authoring Conventions

## Overview

This guide gives the conventions that keep a development document archivable:
assets and links laid out so the document still resolves after the archive
planner relocates it.

The conventions apply to documents under this repository's development root,
`dev/`. What archival does with an artifact follows from the class of the area
holding it, which the configuration reference defines under
[Development-area classification](../docs/reference/configuration.md#development-area-classification);
the [development documentation index](index.md) covers where a new document
goes. Product documentation under `docs/` lies outside the development root, so
a plan retains it where it is rather than scheduling a destination for it, even
when a development document links it.

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
<area>/
  <issue-directory>/
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
- Assets are scheduled in the same plan as their document
- Easy to validate and archive together
- Clear ownership and organization
- Relative links remain valid after moving

**On archival:** the planner discovers the assets a document references and
carries them in the same plan, and the mirror preserves their arrangement
relative to the document, so the links still resolve at the destination. Preview
the plan, then execute it:

```bash
jit archive document <area>/<issue-directory>/authentication-design.md
jit archive document <area>/<issue-directory>/authentication-design.md --execute
```

### Pattern 2: Shared Assets (Use Sparingly)

Only use shared assets when truly needed by multiple documents:

```
<shared-asset-area>/
  system-architecture.png
<area>/
  <issue-directory>/
    feature-a-design.md
  <other-issue-directory>/
    feature-b-design.md
```

**Link with root-relative paths:**
```markdown
![System Architecture](/<shared-asset-area>/system-architecture.png)
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
See [design doc](../<sibling-issue-directory>/design.md).
```

**4. Root-relative links (for shared assets and cross-area references):**
```markdown
See [architecture](/dev/architecture/core-system-design.md) for context.
![Shared Diagram](/<shared-asset-area>/system-overview.png)
```

### ⚠️ Risky Patterns

**Deep relative traversal (2+ parent levels):**
```markdown
![Asset](../../diagrams/shared/diagram.png)
```

**Why risky:** Moving the document breaks the link. The validator will warn about these.

**Better approach:**
- Use per-doc assets pattern instead, OR
- Use root-relative paths for intentionally shared assets and for anything
  outside the issue's own directory

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
        "document": "<area>/<issue-directory>/design.md",
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

The dependency-aware archive family previews a complete artifact plan before it
mutates the repository:

```bash
# Preview one document and its statically reachable bundle
jit archive document <area>/<issue-directory>/feature.md

# Execute only when the recomputed plan is eligible
jit archive document <area>/<issue-directory>/feature.md --execute

# Evaluate all artifacts owned by a terminal container
jit archive container <container-id>
```

The planner discovers supported Markdown, HTML, and CSS dependencies, classifies
each artifact from reference ownership and the class of the area holding it, and
reports blockers and warnings. Destinations mirror repository-relative source
paths beneath the plan's destination root, which is what carries an author's
relative links through the move intact. Execution rechecks the plan under the
repository write guard, publishes without overwriting, updates exact issue
references, records the durable archive event, and only then attempts
identity-guarded source deletion.
[Archive planning and execution](../docs/reference/cli-commands.md#archive-planning-and-execution)
is the reference for the whole family, including how a container's destination
root is named.

## Examples

### Example 1: Clean Document

**File:** `<area>/<issue-directory>/auth-design.md`
```markdown
# Authentication Design

![Auth Flow](auth-design/auth-flow.png)

See [implementation notes](implementation-notes.md).
```

**Validation:**
```bash
$ jit doc check-links --scope issue:<issue-id>
✅ All documents valid!
Summary: 1 document(s) checked, 0 error(s), 0 warning(s)
```

**Result:** Safe to archive

### Example 2: Document with Warnings

**File:** `<area>/<issue-directory>/feature.md`
```markdown
# Feature X Design

![Old Diagram](../../old-diagrams/diagram.png)
![External](https://example.com/reference.png)
```

**Validation:**
```bash
$ jit doc check-links --scope issue:<issue-id>
⚠️  Warnings (2):
  <area>/<issue-directory>/feature.md (risky_asset_path): Deep relative path '../../old-diagrams/diagram.png' may break if document is moved
  <area>/<issue-directory>/feature.md (external_asset): External URL (not validated): https://example.com/reference.png

Summary: 1 document(s) checked, 0 error(s), 2 warning(s)
```

**Result:** Review warnings, consider fixing before archival

### Example 3: Document with Errors

**File:** `<area>/<issue-directory>/broken.md`
```markdown
# Broken Document

![Missing](broken/gone.png)
See [nonexistent](missing.md).
```

**Validation:**
```bash
$ jit doc check-links --scope issue:<issue-id>
❌ Errors found (2):
  <area>/<issue-directory>/broken.md (missing_asset): Asset not found: broken/gone.png
  <area>/<issue-directory>/broken.md (broken_link): Document 'missing.md' not found (resolved to <area>/<issue-directory>/missing.md)

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

1. **Create the document where its issue owns it** — `mkdir -p "$(jit doc dir <issue-id> <area>)"`, then `jit doc add <issue-id> <path>`
2. **Follow per-doc assets pattern** for new documents
3. **Name asset directory after document** (e.g., `my-design.md` → `my-design/`)
4. **Use relative links** for per-doc assets
5. **Use root-relative links** for intentionally shared assets
6. **Avoid deep relative traversal** (2+ `../`)

### Before Archival

1. **Validate links:** `jit doc check-links --scope issue:<id>`
2. **Fix errors** if exit code 1
3. **Review warnings** if exit code 2
4. **Review the plan:** `jit archive document <path> --json`
5. **Execute an eligible plan:** `jit archive document <path> --execute`

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

- [Development Documentation Index](index.md) - Areas, lifecycle, and where a new document goes
- [Documentation configuration](../docs/reference/configuration.md#documentation) - Area classification and issue artifact directories
- [Archive planning and execution](../docs/reference/cli-commands.md#archive-planning-and-execution) - The archive command family
- [Product Documentation](../docs/index.md) - User-facing documentation
