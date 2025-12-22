# Documentation Lifecycle Phase 2: Ergonomics & Automation

## Overview

Phase 2 builds on the asset management foundations from Phase 1, adding automation, link rewriting, and quality-of-life improvements. This phase transforms the documentation lifecycle from a manual process requiring careful validation into a streamlined, automated system that handles common scenarios with minimal user intervention.

## Motivation

**Phase 1 delivers:**
- Asset discovery and tracking
- Link validation
- Safe manual archival
- Basic snapshot export

**Phase 1 limitations:**
- Manual archival (one doc at a time)
- Cannot handle shared assets with relative links
- No automation of archival based on issue state
- Limited format support (Markdown only)
- Manual cleanup of old docs

**Phase 2 addresses:**
- Batch archival operations
- Link rewriting for shared assets
- Automated sweep based on policies
- Additional format adapters
- LFS pointer handling
- Improved ergonomics

## Goals

1. **Automation**: Reduce manual work for archival operations
2. **Safety**: Enable archival scenarios currently blocked
3. **Flexibility**: Support multiple document formats
4. **Productivity**: Batch operations for large-scale cleanup
5. **Robustness**: Handle edge cases (LFS, large files, complex links)

## Scope

### In Scope

**Link Rewriting Engine:**
- Parse document content using format adapters
- Compute new paths for moved assets
- Rewrite links atomically with validation
- Preserve link semantics (relative vs absolute)

**Batch Archival (jit doc sweep):**
- Query eligible docs based on criteria
- Preview batch operations with dry-run
- Execute multiple archival operations atomically
- Rollback on any failure

**Asset Normalization (jit doc assets collect):**
- Convert shared assets to per-doc layout
- Update all references across docs
- Maintain link integrity

**Additional Format Adapters:**
- AsciiDoc support (widely used in documentation)
- Optional: reStructuredText (Python ecosystem)
- Optional: MDX (React documentation)

**LFS Handling:**
- Detect LFS pointers
- Validate working tree content availability
- Policy-based behavior (require, allow, prefer)

**Archival Automation:**
- Policy-based automatic archival
- Trigger on issue state transitions
- Configurable retention periods
- Dry-run preview before execution

### Out of Scope (Future)

- External asset download automation
- Rendering/publishing pipelines
- Cross-repository dependencies
- WYSIWYG editing integration
- Automated format conversion
- Version control integration beyond git

## Design

### 1. Link Rewriting Engine

#### Architecture

```rust
pub struct LinkRewriter {
    adapter: &dyn DocFormatAdapter,
    mapping: HashMap<String, String>,  // old_path -> new_path
}

impl LinkRewriter {
    /// Rewrite links in document content
    pub fn rewrite(&self, content: &str) -> Result<String> {
        // Delegate to adapter for format-specific rewriting
        self.adapter.rewrite_links(content, &self.mapping)
    }
    
    /// Compute link mapping for an archival operation
    pub fn compute_mapping(
        doc_old: &Path,
        doc_new: &Path,
        assets: &[AssetReference],
    ) -> HashMap<String, String> {
        // Map old asset paths to new paths
        // Preserve relative link semantics
    }
}
```

#### Algorithm

1. **Parse current links**: Extract all links from document
2. **Compute new paths**: Map old paths to archive destinations
3. **Validate mapping**: Ensure no broken links after rewrite
4. **Apply rewrite**: Update document content atomically
5. **Verify**: Re-parse and validate links still resolve

#### Safety

- **Validate before write**: Ensure new links resolve
- **Atomic updates**: Write temp file, verify, then replace
- **Preserve semantics**: Relative links stay relative
- **Rollback on error**: No partial rewrites

#### Use Cases

```bash
# Archive with shared asset rewrite
jit doc archive docs/active/feature.md --rewrite-links

# Normalize shared to per-doc
jit doc assets collect docs/active/feature.md --move
```

### 2. Batch Archival (jit doc sweep)

#### Command Interface

```bash
jit doc sweep [OPTIONS]

Options:
  --since-release <TAG>    Archive docs linked to done issues since release
  --since-date <DATE>      Archive docs linked to done issues since date
  --older-than <DAYS>      Archive docs completed more than N days ago
  --category <TYPE>        Filter by document type
  --dry-run                Show what would be archived
  --yes                    Skip confirmation prompt
  --parallel <N>           Process N docs concurrently (default: 1)
```

#### Algorithm

```rust
pub fn sweep_archives(
    storage: &Storage,
    criteria: &SweepCriteria,
    dry_run: bool,
) -> Result<SweepReport> {
    // 1. Query eligible documents
    let candidates = find_archival_candidates(storage, criteria)?;
    
    // 2. Validate each candidate
    let validated = candidates.into_iter()
        .filter_map(|doc| validate_archival(&doc).ok())
        .collect::<Vec<_>>();
    
    // 3. Preview plan
    if dry_run {
        return Ok(preview_plan(&validated));
    }
    
    // 4. Execute batch archival
    let results = archive_batch(&validated)?;
    
    // 5. Report results
    Ok(SweepReport::from(results))
}
```

#### Selection Criteria

```rust
pub struct SweepCriteria {
    /// Archive docs from done issues since this release tag
    pub since_release: Option<String>,
    
    /// Archive docs from done issues since this date
    pub since_date: Option<DateTime>,
    
    /// Archive docs completed more than N days ago
    pub older_than_days: Option<u32>,
    
    /// Filter by document type
    pub doc_type: Option<String>,
    
    /// Custom filter predicate
    pub filter: Option<Box<dyn Fn(&Document) -> bool>>,
}
```

#### Atomic Batch Execution

- **Plan first**: Validate all operations before executing
- **Execute in transaction**: All succeed or all rollback
- **Progress reporting**: Show progress for large batches
- **Partial success handling**: Mark successful vs failed

#### Example Output

```
Planning archival sweep...

Criteria:
  Since release: v1.0
  Document type: design
  Issues: done state only

Found 15 candidates:
  ✓ docs/active/webhooks-design.md → .jit/docs/archive/features/
  ✓ docs/active/oauth-design.md → .jit/docs/archive/features/
  ✗ docs/active/api-gateway.md (relative shared links, needs --rewrite-links)
  ...

Eligible: 12 documents
Skipped: 3 documents (validation failures)

Execute archival? (y/n)
```

### 3. Asset Normalization (jit doc assets collect)

#### Purpose

Convert documents using shared assets with relative links to per-doc asset layout, enabling safe archival.

#### Command Interface

```bash
jit doc assets collect <ISSUE-ID> <PATH> [OPTIONS]

Options:
  --dest <DIR>       Destination for per-doc assets (default: <doc>_assets/)
  --copy             Copy assets (default)
  --move             Move assets (error if other docs reference)
  --rewrite-refs     Update all docs referencing these assets
  --dry-run          Show plan without executing
```

#### Algorithm

1. **Scan document**: Find all asset references
2. **Classify assets**: Per-doc vs shared
3. **For each shared asset**:
   - Find all referencing documents
   - Compute new per-doc path
   - Copy or move asset to per-doc location
   - Update references in all docs
4. **Validate**: All links still resolve
5. **Commit changes**: Atomic update

#### Safety

- **Check usage**: Error if moving shared asset still in use
- **Update all refs**: Find and update all documents
- **Validate result**: Ensure no broken links
- **Atomic**: All-or-nothing operation

### 4. Additional Format Adapters

#### AsciiDoc Adapter

```rust
pub struct AsciiDocAdapter;

impl DocFormatAdapter for AsciiDocAdapter {
    fn id(&self) -> &'static str { "asciidoc" }
    
    fn supports_path(&self, path: &str) -> bool {
        path.ends_with(".adoc") || path.ends_with(".asciidoc")
    }
    
    fn scan_assets(&self, doc_dir: &Path, content: &str) -> Result<Vec<String>> {
        // Parse AsciiDoc syntax:
        // image::path[alt text]
        // image:path[alt text] (inline)
        // include::path[]
    }
    
    fn rewrite_links(&self, content: &str, mapping: &[(String, String)]) -> Result<String> {
        // Rewrite AsciiDoc link syntax
    }
}
```

#### reStructuredText Adapter (Optional)

```rust
pub struct ReStructuredTextAdapter;

impl DocFormatAdapter for ReStructuredTextAdapter {
    fn id(&self) -> &'static str { "restructuredtext" }
    
    fn supports_path(&self, path: &str) -> bool {
        path.ends_with(".rst")
    }
    
    fn scan_assets(&self, doc_dir: &Path, content: &str) -> Result<Vec<String>> {
        // Parse reST syntax:
        // .. image:: path
        // .. figure:: path
        // .. include:: path
    }
}
```

#### MDX Adapter (Optional)

For React documentation:
```rust
pub struct MdxAdapter;

impl DocFormatAdapter for MdxAdapter {
    fn id(&self) -> &'static str { "mdx" }
    
    fn supports_path(&self, path: &str) -> bool {
        path.ends_with(".mdx")
    }
    
    fn scan_assets(&self, doc_dir: &Path, content: &str) -> Result<Vec<String>> {
        // Parse MDX syntax:
        // Markdown image syntax
        // JSX <img src="path" />
        // import statements
    }
}
```

### 5. LFS Handling

#### Detection

```rust
pub fn detect_lfs_pointer(path: &Path) -> Result<Option<LfsPointer>> {
    let content = fs::read_to_string(path)?;
    
    if content.starts_with("version https://git-lfs.github.com/spec/") {
        // Parse LFS pointer
        Ok(Some(LfsPointer::parse(&content)?))
    } else {
        Ok(None)
    }
}

pub struct LfsPointer {
    pub version: String,
    pub oid: String,
    pub size: u64,
}
```

#### Policy Configuration

```toml
[documentation.assets.lfs]
policy = "allow-pointers"  # require | allow-pointers | prefer-working-tree

# require: Fail if LFS content not available in working tree
# allow-pointers: Include LFS pointer files in snapshots
# prefer-working-tree: Use working tree if available, fall back to pointer
```

#### Behavior

**Snapshot Export:**
- `require`: Fail if asset is LFS pointer without working tree content
- `allow-pointers`: Include pointer file, warn in manifest
- `prefer-working-tree`: Use actual content if available

**Asset Scanning:**
- Detect LFS pointers
- Compute hash of pointer file (not content)
- Mark asset as LFS in metadata

### 6. Archival Automation

#### Trigger Points

```rust
pub enum ArchivalTrigger {
    /// Manual invocation
    Manual,
    
    /// Issue state transition to Done
    IssueCompleted,
    
    /// Issue state transition to Rejected
    IssueRejected,
    
    /// Periodic sweep (cron-like)
    Scheduled,
    
    /// After release tagging
    ReleaseCreated,
}
```

#### Policy Configuration

```toml
[documentation]
mode = "release"  # manual | release | done

# When mode = "release"
retention_releases = 2  # Keep docs for 2 releases after completion

# When mode = "done"
retention_days = 30  # Keep docs for 30 days after issue done

# Automatic sweep
[documentation.auto_sweep]
enabled = false
trigger = "release"  # release | daily | weekly
```

#### Implementation

```rust
pub fn on_issue_state_change(
    storage: &Storage,
    issue_id: &str,
    new_state: State,
) -> Result<()> {
    if new_state == State::Done || new_state == State::Rejected {
        let config = storage.config().documentation();
        
        if config.mode() == "done" {
            // Queue for archival after retention period
            schedule_archival(issue_id, config.retention_days())?;
        }
    }
    Ok(())
}
```

## Implementation Plan

### Task Breakdown

**1. Link Rewriting Engine (5 days)**
- Implement LinkRewriter core
- Add rewrite_links() to Markdown adapter
- Integration tests with various link patterns
- Edge case handling (anchors, complex paths)

**2. jit doc assets collect (3 days)**
- Asset collection logic
- Reference updating across docs
- Copy vs move semantics
- Validation and rollback

**3. jit doc sweep (5 days)**
- Candidate selection with criteria
- Batch validation
- Atomic batch execution
- Progress reporting and dry-run

**4. AsciiDoc Adapter (3 days)**
- AsciiDoc syntax parsing
- Asset extraction
- Link rewriting
- Integration tests

**5. LFS Handling (2 days)**
- LFS pointer detection
- Policy implementation
- Snapshot export integration

**6. Optional: Additional Adapters (3 days each)**
- reStructuredText adapter
- MDX adapter

**7. Automation Framework (4 days)**
- Trigger system
- Policy configuration
- Scheduled sweep
- Integration with issue lifecycle

**Total:** ~25-30 days (1-1.5 months with one developer)

## Success Metrics

### Phase 2 Complete When:

1. **Link rewriting functional**
   - Can archive docs with relative shared links
   - No broken links after rewrite
   - Validation catches issues

2. **Batch operations working**
   - jit doc sweep processes 100+ docs
   - Atomic rollback on errors
   - Clear progress reporting

3. **Multiple formats supported**
   - Markdown (Phase 1)
   - AsciiDoc (Phase 2)
   - Optional: reST, MDX

4. **LFS handled correctly**
   - Policies enforced
   - Snapshots handle LFS appropriately
   - Clear warnings/errors

5. **Automation operational**
   - Auto-archival based on policy
   - Scheduled sweeps
   - Triggered on issue state changes

6. **Quality maintained**
   - Zero link breakage in production use
   - Performance acceptable (100s of docs)
   - Clear error messages

## Testing Strategy

### Unit Tests

- Link rewriting for each adapter
- Mapping computation correctness
- Policy evaluation
- LFS pointer parsing

### Integration Tests

- Batch archival with rollback
- Cross-document reference updating
- Format adapter interaction
- Snapshot export with LFS

### Property-Based Tests

- Link rewriting preserves reachability
- Batch operations are atomic
- No path escapes after rewrite

### Performance Tests

- Sweep 1000+ documents
- Parallel archival
- Large file handling (LFS)

## Configuration Schema

```toml
[documentation]
mode = "manual"  # manual | release | done
archive_root = ".jit/docs/archive"
retention_releases = 2
retention_days = 30

[documentation.sweep]
enabled = false
schedule = "after-release"  # after-release | daily | weekly
dry_run_first = true

[documentation.assets]
mode = "mixed"
per_doc_suffix = "_assets"
shared_roots = ["docs/assets/"]

[documentation.assets.lfs]
policy = "allow-pointers"  # require | allow-pointers | prefer-working-tree

[documentation.formats]
markdown = true
asciidoc = true
restructuredtext = false
mdx = false
```

## Migration from Phase 1

Phase 2 is backward compatible:
- Phase 1 commands continue to work
- New commands are additive
- Configuration is optional (sensible defaults)
- Existing docs unaffected

## Open Questions

1. **Parallelization**: How many concurrent archival operations?
   - Recommendation: Start with serial, add --parallel flag later

2. **Link rewriting complexity**: Support complex link patterns?
   - Recommendation: Start with simple relative/absolute, iterate

3. **Format adapter priority**: Order of implementation?
   - Recommendation: AsciiDoc first (widely used), then reST/MDX

4. **Automation triggers**: Git hooks vs polling?
   - Recommendation: Polling initially, git hooks in Phase 3

5. **Error recovery**: Retry failed operations?
   - Recommendation: Manual retry initially, auto-retry later

## Dependencies

- Phase 1 complete (all 7 tasks)
- Milestone v1.0 shipped
- User feedback on Phase 1 ergonomics
- Real-world usage data on edge cases

## Risks

1. **Link rewriting bugs**: Complex to get right
   - Mitigation: Extensive testing, conservative approach

2. **Performance**: Batch operations on large repos
   - Mitigation: Profile early, optimize hot paths

3. **Format adapter complexity**: Each format has quirks
   - Mitigation: Start with well-documented formats (AsciiDoc)

4. **LFS edge cases**: Many LFS configurations
   - Mitigation: Clear policies, good error messages

5. **Automation errors**: Auto-archival could break things
   - Mitigation: Dry-run by default, extensive validation

## Alternatives Considered

### Link Rewriting

**Alt 1: External tool integration**
- Pros: Leverage existing tools
- Cons: Complex integration, less control
- Decision: Custom implementation for tighter integration

**Alt 2: Prohibit relative shared links**
- Pros: Simpler, no rewriting needed
- Cons: User frustration, limited flexibility
- Decision: Support rewriting for better UX

### Batch Operations

**Alt 1: Script wrapper around single operations**
- Pros: Simpler implementation
- Cons: No atomicity, poor error handling
- Decision: Native batch support for safety

## Future Enhancements (Phase 3+)

- External asset download automation
- Git hook integration for triggers
- Archive compression (gzip, zstd)
- Archive to cloud storage
- Document format conversion
- Enhanced snapshot formats (zip, custom)
- Performance optimization (parallel, caching)
- Web UI for archival operations
