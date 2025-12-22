# Search Implementation

**Date**: 2025-12-04  
**Status**: In Progress  
**Approach**: Ripgrep (Phase 1), Optional Tantivy (Phase 2)

## Overview

Full-text search across issues and referenced documents using `ripgrep` as the search backend. This provides fast, simple search without index management. Tantivy can be added later for advanced features if needed.

## Architecture Decision

### Phase 1: Ripgrep (Current)

**Why Ripgrep First:**
- ✅ Zero dependencies (system tool)
- ✅ Extremely fast (optimized C, SIMD)
- ✅ Simple implementation (~200 lines)
- ✅ No index to manage or sync
- ✅ Always searches current state
- ✅ Rich features (regex, globs, context)
- ✅ Built-in JSON output
- ✅ Git-aware by default

**Implementation:**
```rust
// Shell out to ripgrep with --json flag
Command::new("rg")
    .arg("--json")
    .arg(query)
    .arg(data_dir)
    .output()
```

**Graceful Degradation:**
- Check for `rg` binary with `which::which("rg")`
- Return user-friendly error if not installed
- Suggest installation: `https://github.com/BurntSushi/ripgrep`
- Never panic, always `Result<T, Error>`

### Phase 2: Tantivy (Future, Optional)

**When to Add Tantivy:**
- Repo grows to >1000 issues
- Users need relevance ranking
- Need faceted filtering (state/priority)
- Want fuzzy matching or complex queries
- Performance becomes issue with ripgrep

**Hybrid Design:**
```rust
pub enum SearchBackend {
    Ripgrep,   // Default, always available
    Tantivy,   // Optional, feature-gated
}

impl CommandExecutor {
    fn search_backend(&self) -> SearchBackend {
        if self.has_tantivy_index() {
            SearchBackend::Tantivy
        } else {
            SearchBackend::Ripgrep
        }
    }
}
```

**Feature Flag:**
```toml
[features]
default = []
tantivy-search = ["tantivy"]

[dependencies]
tantivy = { version = "0.22", optional = true }
```

## Data Flow

```
User Query
    ↓
CLI: jit search "authentication"
    ↓
CommandExecutor::search()
    ↓
search::ripgrep_search()
    ↓
Command::new("rg")
    --json
    --query "authentication"
    .jit/
    ↓
Parse JSON output
    ↓
Extract issue IDs from paths
    ↓
Return Vec<SearchResult>
    ↓
Format output (human or JSON)
```

## API Design

### Search Module (`search.rs`)

```rust
/// Search result from ripgrep
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub issue_id: Option<String>,  // From filename
    pub path: String,
    pub line_number: u64,
    pub line_text: String,
    pub matches: Vec<SearchMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Search configuration
#[derive(Debug, Default)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub regex: bool,
    pub context_lines: usize,
    pub max_results: Option<usize>,
    pub file_pattern: Option<String>,  // "*.json", "*.md"
}

/// Execute ripgrep search
pub fn search(
    data_dir: &Path,
    query: &str,
    options: SearchOptions,
) -> Result<Vec<SearchResult>>;
```

### CLI Interface

```bash
jit search <query> [OPTIONS]

OPTIONS:
    -r, --regex              Use regex pattern matching
    -C, --case-sensitive     Case sensitive search
    -c, --context <N>        Show N lines of context
    -n, --limit <N>          Maximum results to return
    -g, --glob <PATTERN>     Search only matching files (e.g., "*.json")
        --json               Output as JSON

EXAMPLES:
    # Basic search
    jit search "authentication"
    
    # Regex search
    jit search "auth(entication|orization)" --regex
    
    # Search only issues
    jit search "priority.*high" --glob "*.json"
    
    # Search only documents
    jit search "API design" --glob "*.md"
    
    # With context
    jit search "JWT" --case-sensitive --context 2
    
    # JSON output for scripts
    jit search "login" --json
```

### MCP Tool

```typescript
{
  name: "search_issues",
  description: "Search issues and documents using ripgrep",
  inputSchema: {
    query: string,
    regex?: boolean,
    case_sensitive?: boolean,
    context?: number,
    limit?: number,
    glob?: string
  }
}
```

## Output Formats

### Human-Readable

```
Search results for "authentication" (5 matches):

Issue abc123 | Implement user authentication
  Line 3: "title": "Implement user authentication",
  Line 7: "description": "Add JWT authentication middleware"

Issue def456 | Add rate limiting
  Line 12: "context": { "auth_method": "JWT" }

Document docs/api-design.md | Issue abc123
  Line 45: The authentication flow uses OAuth 2.0 with PKCE

(5 matches in 2 issues, 1 document)
Search completed in 12ms
```

### JSON Output

```json
{
  "query": "authentication",
  "options": {
    "case_sensitive": false,
    "regex": false,
    "context_lines": 0
  },
  "total": 5,
  "results": [
    {
      "issue_id": "abc123",
      "path": ".jit/issues/abc123.json",
      "line_number": 3,
      "line_text": "  \"title\": \"Implement user authentication\",\n",
      "matches": [
        {
          "text": "authentication",
          "start": 20,
          "end": 34
        }
      ]
    }
  ],
  "stats": {
    "duration_ms": 12,
    "issues_matched": 2,
    "documents_matched": 1
  }
}
```

## Error Handling

### Ripgrep Not Installed

**Error:**
```
Error: ripgrep (rg) is not installed

Ripgrep is required for search functionality.

Install ripgrep:
  - Ubuntu/Debian: apt install ripgrep
  - macOS: brew install ripgrep
  - Windows: choco install ripgrep
  - Other: https://github.com/BurntSushi/ripgrep#installation
```

**JSON:**
```json
{
  "error": {
    "code": "RIPGREP_NOT_FOUND",
    "message": "ripgrep (rg) is not installed",
    "suggestion": "Install from https://github.com/BurntSushi/ripgrep"
  }
}
```

### No Matches

**Exit code**: 0 (not an error)

**Output:**
```
No matches found for "nonexistent"
```

**JSON:**
```json
{
  "query": "nonexistent",
  "total": 0,
  "results": []
}
```

### Invalid Regex

**Exit code**: 2

**Error:**
```
Error: Invalid regex pattern

The regex pattern is malformed: unclosed group

Query: "auth(entication"
         ^^^^^^^^^^^^^^^
```

## Performance Characteristics

### Ripgrep Performance

| Repo Size | Search Time | Notes |
|-----------|-------------|-------|
| 10 issues | <10ms | Instant |
| 100 issues | <50ms | Very fast |
| 1,000 issues | <200ms | Fast |
| 10,000 issues | <2s | May need Tantivy |

### Tantivy Performance (Future)

| Repo Size | Index Time | Search Time |
|-----------|------------|-------------|
| 10 issues | ~50ms | <10ms |
| 100 issues | ~500ms | <20ms |
| 1,000 issues | ~5s | <50ms |
| 10,000 issues | ~50s | <100ms |

**Trade-off**: Ripgrep has no index overhead but slower on huge repos. Tantivy has index overhead but constant-time search.

## Testing Strategy

### Unit Tests (~10 tests)

**Test file**: `crates/jit/src/search.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_ripgrep_json_match() {
        // Parse single match event
    }
    
    #[test]
    fn test_parse_ripgrep_json_multiple_matches() {
        // Parse multiple match events
    }
    
    #[test]
    fn test_extract_issue_id_from_path() {
        // "issues/abc123.json" -> Some("abc123")
    }
    
    #[test]
    fn test_extract_issue_id_from_document_path() {
        // "docs/design.md" -> None
    }
    
    #[test]
    fn test_search_options_default() {
        // Default options
    }
    
    #[test]
    fn test_build_ripgrep_command_basic() {
        // Basic query builds correct args
    }
    
    #[test]
    fn test_build_ripgrep_command_with_options() {
        // All options set correctly
    }
    
    #[test]
    fn test_search_result_serialization() {
        // JSON round-trip
    }
    
    #[test]
    fn test_ripgrep_not_installed() {
        // Graceful error when rg missing
    }
    
    #[test]
    fn test_ripgrep_no_matches() {
        // Exit code 1 returns empty vec, not error
    }
}
```

### Integration Tests (~8 tests)

**Test file**: `crates/jit/tests/search_tests.rs`

```rust
use jit::test_harness::TestHarness;

#[test]
fn test_search_finds_issue_by_title() {
    let harness = TestHarness::new();
    harness.create_issue("Authentication", "Add JWT auth");
    
    let results = harness.search("Authentication", Default::default());
    assert_eq!(results.len(), 1);
    assert!(results[0].line_text.contains("Authentication"));
}

#[test]
fn test_search_finds_issue_by_description() {
    // Search in description field
}

#[test]
fn test_search_with_regex() {
    // Regex pattern matching
}

#[test]
fn test_search_with_glob_filter() {
    // Filter by file pattern
}

#[test]
fn test_search_with_context() {
    // Show context lines
}

#[test]
fn test_search_limit_results() {
    // Respect max results
}

#[test]
fn test_search_case_sensitive() {
    // Case sensitivity
}

#[test]
fn test_search_in_documents() {
    // Search referenced markdown files
}
```

### CLI Tests (~5 tests)

**Test file**: `crates/jit/tests/cli_search_tests.rs`

```rust
use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn test_search_command_basic() {
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.arg("search")
       .arg("test")
       .assert()
       .success();
}

#[test]
fn test_search_command_json_output() {
    // --json flag produces valid JSON
}

#[test]
fn test_search_command_with_options() {
    // All CLI options work
}

#[test]
fn test_search_command_no_matches() {
    // Exit 0 with empty results
}

#[test]
fn test_search_command_ripgrep_missing() {
    // Graceful error message
}
```

### MCP Tests (~3 tests)

**Test file**: `mcp-server/test.js`

```javascript
describe('Search MCP Tool', () => {
  test('search_issues tool exists', () => {
    // Tool registered with correct schema
  });
  
  test('search executes with basic query', async () => {
    // Returns valid results
  });
  
  test('search handles no matches', async () => {
    // Returns empty array, not error
  });
});
```

**Total Tests**: ~26 tests

## Implementation Checklist

**Phase 1: Core Module** (1 hour)
- [ ] Create `crates/jit/src/search.rs`
- [ ] Add `which = "6.0"` dependency
- [ ] Define `SearchResult` and `SearchMatch` types
- [ ] Define `SearchOptions` struct
- [ ] Write 10 unit tests (TDD)
- [ ] Implement `search()` function
- [ ] Implement `parse_ripgrep_json()`
- [ ] Implement `extract_issue_id()`
- [ ] Handle ripgrep not installed gracefully
- [ ] Export module in `lib.rs`

**Phase 2: CLI Commands** (1 hour)
- [ ] Add `Search` command to `cli.rs`
- [ ] Implement handler in `commands.rs`
- [ ] Add human-readable output formatting
- [ ] Add JSON output support
- [ ] Write 8 integration tests
- [ ] Write 5 CLI tests
- [ ] Update help documentation

**Phase 3: MCP Integration** (30 min)
- [ ] Add `search_issues` tool to `mcp-server/src/index.ts`
- [ ] Update MCP documentation
- [ ] Write 3 MCP tests

**Phase 4: Documentation** (30 min)
- [ ] Update `ROADMAP.md` (Phase 3.1 complete)
- [ ] Update `README.md` with search examples
- [ ] Add section to `docs/design.md`
- [ ] This document (`docs/search-implementation.md`)

**Phase 5: Verification**
- [ ] Run `cargo test` (all tests pass)
- [ ] Run `cargo clippy` (zero warnings)
- [ ] Run `cargo fmt`
- [ ] Test with real repository data
- [ ] Verify MCP tool works with Copilot CLI

## Future Enhancements (Tantivy)

### When to Migrate

**Indicators:**
- Repo has >1000 issues
- Search takes >2 seconds consistently
- Users request relevance ranking
- Need faceted filtering (state/priority)

### Migration Path

1. **Add feature flag**: `[features] tantivy-search = ["tantivy"]`
2. **Implement `TantivySearcher`** alongside `RipgrepSearcher`
3. **Auto-detect backend**: Use Tantivy if index exists, else ripgrep
4. **Migration command**: `jit search migrate-to-tantivy`
5. **Maintain compatibility**: Both backends use same `SearchResult` type

### Tantivy Schema

```rust
// Future Tantivy implementation
let mut schema_builder = Schema::builder();
schema_builder.add_text_field("id", STRING | STORED);
schema_builder.add_text_field("title", TEXT | STORED);
schema_builder.add_text_field("description", TEXT);
schema_builder.add_text_field("content", TEXT);
schema_builder.add_facet_field("state");
schema_builder.add_facet_field("priority");
schema_builder.add_facet_field("assignee");
```

## Success Criteria

- [ ] Can search issues by title, description, context
- [ ] Can search documents by content
- [ ] Regex and glob filtering work
- [ ] Graceful error when ripgrep not installed
- [ ] CLI has `--json` output
- [ ] MCP tool accessible to agents
- [ ] All 26 tests pass
- [ ] Zero clippy warnings
- [ ] Performance: <200ms for 1000 issues

## References

- Ripgrep: https://github.com/BurntSushi/ripgrep
- Ripgrep JSON format: https://docs.rs/grep-printer/latest/grep_printer/struct.JSON.html
- Tantivy (future): https://github.com/quickwit-oss/tantivy
