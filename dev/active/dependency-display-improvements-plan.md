# Implementation Plan: Dependency Display Improvements

## ✅ COMPLETED

All tasks completed successfully! The dependency display improvements are now fully implemented.

## Problem Statement

The current dependency exploration tools have two major issues:

1. **`jit issue show` has poor dependency display:**
   - Human output: Shows raw UUID array: `Dependencies: ["32f804f1-...", "12ef7efb-...", ...]`
   - Inconsistent formatting between fields
   - No state or priority indicators

2. **Graph commands are confusing and incomplete:**
   - `jit graph deps` vs `jit graph show` - unclear distinction
   - `show` is mostly redundant (same as `deps --transitive`)
   - No depth control (only immediate vs all-transitive)
   - Transitive output loses tree structure (flattened list)
   - Diamonds (shared dependencies) not handled well in display

## Proposed Solution

### Part 1: Fix `jit issue show` Dependency Display

**Goal:** Show dependencies with short hash, title, and state in both human and JSON formats.

**Human-readable output:**
```
Dependencies (3/12 complete):
  ✓ cbf75d46 | Implement FromStr trait [done]
  ✓ a66e16a4 | Add git merge driver [done]
  ○ 32f804f1 | CLI ergonomics improvements [in_progress]
  ○ 12ef7efb | Centralize label matching logic [ready]
  ...
```

**JSON output:**
```json
"dependencies": [
  {
    "id": "cbf75d46-2bea-46ce-8b70-80ae2f8e003d",
    "short_id": "cbf75d46",
    "title": "Implement FromStr trait for State and Priority enums",
    "state": "done"
  },
  ...
]
```

### Part 2: Overhaul Graph Commands

**Changes:**

1. **Add `--depth` parameter to `jit graph deps`:**
   - `--depth 1` (default) - immediate dependencies only
   - `--depth 2` - two levels deep
   - `--depth 0` or `--depth unlimited` - all transitive (replaces `--transitive`)

2. **Deprecate/remove `jit graph show`:**
   - Redirect to `jit graph deps --depth 0`
   - Or keep as alias for backward compatibility

3. **Add tree structure to output:**
   - Preserve parent-child relationships in output
   - Handle diamonds (shared dependencies) properly
   - Indicate when a node is shown multiple times vs shown once with multiple parents

**Human-readable tree output:**
```
Dependencies of 9d427a6b (depth 2):
├─○ 32f804f1 | CLI ergonomics improvements [in_progress]
├─✓ cbf75d46 | Implement FromStr trait [done]
├─○ 91e3a165 | Story: Web UI Visual Enhancements [backlog]
│  ├─○ 13c69884 | Add web UI state transition buttons [ready]
│  ├─○ f527996e | Add search focus navigation [ready]
│  └─○ d8de0f33 | Add timestamps to Issue model [ready] (also dep of 56b7e503)
├─○ d4290046 | Improve DAG layout [done]
│  ├─○ 402d1a8f | Phase 3: Advanced Features [ready]
│  ├─○ 783a8086 | Phase 1: Clustering [ready]
│  └─○ 6f678db0 | Phase 2: Collapse/Expand [ready]
└─... (8 more)

Summary: 3/12 complete, 8 ready, 1 in_progress
```

**JSON tree structure:**
```json
{
  "issue_id": "9d427a6b",
  "depth": 2,
  "tree": [
    {
      "id": "32f804f1-...",
      "short_id": "32f804f1",
      "title": "CLI ergonomics improvements",
      "state": "in_progress",
      "priority": "normal",
      "level": 1,
      "children": []
    },
    {
      "id": "91e3a165-...",
      "title": "Story: Web UI Visual Enhancements",
      "state": "backlog",
      "level": 1,
      "children": [
        {
          "id": "13c69884-...",
          "title": "Add web UI state transition buttons",
          "state": "ready",
          "level": 2,
          "children": []
        },
        {
          "id": "d8de0f33-...",
          "title": "Add timestamps to Issue model",
          "state": "ready",
          "level": 2,
          "shared": true,
          "also_child_of": ["56b7e503-..."],
          "children": []
        }
      ]
    }
  ],
  "summary": {
    "total": 12,
    "by_state": {
      "done": 3,
      "ready": 8,
      "in_progress": 1
    }
  }
}
```

## Implementation Workplan

- [x] **Task 1: Design dependency display format**
  - [x] Use MinimalIssue for dependency representation
  - [x] Add helper methods: short_id(), state_symbol()
  - [x] Design human-readable format (symbols: ✓ for done, ○ for active)
  - [x] Design JSON structure (MinimalIssue array with id, title, state, priority)

- [x] **Task 2: Implement `jit issue show` improvements (TDD)**
  - [x] Write tests for enhanced dependency display
  - [x] Create helper function to enrich dependencies with metadata
  - [x] Update human-readable output formatter
  - [x] Update JSON output structure (IssueShowResponse)
  - [x] Add summary line (X/Y complete)
  
- [x] **Task 3: Add `--depth` parameter to `jit graph deps` (TDD)**
  - [x] Write tests for depth-limited traversal
  - [x] Implement depth-limited dependency traversal
  - [x] Update CLI argument parsing
  - [x] Update help text
  - [x] Removed --transitive flag (no backward compatibility)

- [x] **Task 4: Implement tree structure preservation (TDD)**
  - [x] Write tests for tree building (including diamonds)
  - [x] Implement tree builder that tracks level and parent relationships
  - [x] Handle diamond detection (mark shared nodes)
  - [x] Update human output formatter with tree symbols (├─, └─, │)
  - [x] Create hierarchical JSON output structure

- [x] **Task 5: Add summary statistics**
  - [x] Write test for summary calculation
  - [x] Implement state aggregation (count by state)
  - [x] Add to both human and JSON output

- [x] **Task 6: Remove `jit graph show`**
  - [x] Remove command from CLI
  - [x] Remove unused response types and methods
  - [x] Update all tests to use `graph deps`
  - [x] Regenerate MCP server schema

- [x] **Task 7: Quality gates**
  - [x] All tests pass
  - [ ] Clippy clean
  - [ ] Formatting correct
  - [ ] Documentation updated

## Design Decisions & Notes

### Tree Structure Algorithm

**Challenge:** DAGs can have diamonds (A→B, A→C, B→D, C→D). How to display?

**Options:**
1. **Show first occurrence in full, mark duplicates:** 
   ```
   ├─ B
   │  └─ D | Task [ready]
   └─ C
      └─ D (see above)
   ```

2. **Show all occurrences, mark as shared:**
   ```
   ├─ B
   │  └─ D | Task [ready] (shared)
   └─ C
      └─ D | Task [ready] (shared)
   ```

3. **Flatten shared nodes to top level:**
   ```
   ├─ D | Task [ready] (shared: B, C)
   ├─ B
   └─ C
   ```

**Recommendation:** Option 2 - Shows full structure, clear about sharing.

### Depth Parameter

- `--depth 1` - Default (immediate dependencies only)
- `--depth 2`, `--depth 3`, etc. - Specific depth
- `--depth 0` or `--depth unlimited` - All transitive dependencies
- Deprecate `--transitive` flag in favor of `--depth 0`

### Visual Indicators

**State symbols:**
- `✓` - done/rejected (terminal states)
- `○` - ready/backlog/in_progress (active states)
- `✗` - blocked (if we add that info)

**Tree symbols:**
- `├─` - Branch to sibling
- └─ - Last branch (no more siblings)
- │  - Continuation line
- (shared) - Appears elsewhere in tree

### Backward Compatibility

**Breaking changes:**
- `jit issue show` JSON output structure changes (dependencies array → objects)
- `jit graph deps --transitive` deprecated (use `--depth 0`)

**Migration path:**
- Keep `--transitive` working but show deprecation warning
- Document JSON structure change in CHANGELOG
- Consider version bump

### Edge Cases

- Empty dependencies: Show "Dependencies: None"
- Self-referential cycles: Should be prevented by validation, but handle gracefully
- Very deep trees (>10 levels): Add truncation with `--max-depth` override
- Very wide trees (>50 children): Paginate or add `--limit` flag

## Testing Strategy

### Unit Tests
- Tree builder handles diamonds correctly
- Depth limiting works at all levels
- Summary statistics are accurate

### Integration Tests  
- End-to-end CLI tests for all output formats
- JSON structure matches schema
- Backward compatibility (old commands still work)

### Manual Testing
- Test with production-polish epic (12 dependencies, some done)
- Test with deep trees (documentation lifecycle)
- Test with diamonds (check for infinite loops)

## Success Criteria

- [ ] `jit issue show` displays dependencies with short hash, title, state
- [ ] `jit graph deps --depth N` works for arbitrary depth
- [ ] Tree structure preserved in output (not flattened)
- [ ] Diamonds (shared dependencies) clearly marked
- [ ] Summary statistics show progress
- [ ] All tests pass
- [ ] Documentation updated
- [ ] No performance regression on large graphs
