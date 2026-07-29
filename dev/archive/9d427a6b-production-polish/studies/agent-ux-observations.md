# Agent UX Observations: Using JIT to Investigate Issues

**Date:** 2026-02-17
**Context:** AI agent (Claude Code) used jit MCP tools to investigate epic `9d427a6b` (Production Polish) and assess what work remains. These observations capture friction points and what worked well from an agent's perspective.

## Task

Understand issue `9d427a6b`, its dependency tree, what's complete, and what still blocks it. Report findings to the user.

## Tools Used

1. `jit issue show 9d427a6b --json` — primary source of truth
2. `jit graph deps 9d427a6b --json` — dependency tree
3. `jit graph downstream 9d427a6b --json` — downstream impact
4. `jit issue show 32f804f1 --json` — drill into incomplete dependency
5. `jit issue show 20a895cc --json` — drill into incomplete dependency

Total: 5 tool calls to build a complete picture.

## What Worked Well

### Inline dependencies in `issue show`
The `dependencies` field in `jit issue show --json` includes state, title, labels, and short IDs for each dependency. This single call provided ~80% of the information needed. An agent doesn't have to make N additional calls to understand the dependency landscape.

### Short IDs
8-character short IDs (`9d427a6b` instead of `9d427a6b-a8f7-4478-a0e1-1637781e00f6`) reduce token usage and cognitive load. They work consistently across all commands.

### JSON output consistency
All commands returned well-structured JSON. Fields were predictable. No parsing surprises.

### `graph downstream` for impact assessment
One call immediately answered "what does this epic block?" — essential for prioritization.

## Pain Points

### 1. `graph deps` header count mismatch
`jit graph deps 9d427a6b` returned the header text "0 dependencies" but the JSON `tree` array contained 13 entries and `summary.total` was 13. The header count appears to be a bug.

**Suggestion:** Fix the display count to match actual results.

### 2. `graph deps` at depth 1 is redundant with `issue show`
At depth 1, `graph deps` returns essentially the same dependency information that `issue show` already includes inline. The real value of `graph deps` is recursive traversal (depth > 1), but depth 1 is the default.

**Suggestion:** Consider making `graph deps` default to unlimited depth, or document that depth 1 is equivalent to the dependencies in `issue show`.

### 3. No filtered dependency view
To find what's still blocking an epic, an agent must:
1. Fetch all dependencies
2. Filter for non-terminal states (not done/rejected)
3. Drill into each incomplete dependency separately

This took 5 calls. A filter like `jit graph deps <id> --incomplete` or `--state-not done,rejected` would reduce this to 1-2 calls.

**Suggestion:** Add `--incomplete` flag to `graph deps` that filters out done/rejected nodes and their resolved subtrees.

### 4. No recursive detail in one call
`graph deps --depth 0` provides a full tree, but only with summary info (id, title, state). To get full details (description, sub-dependencies, gates) for incomplete items, separate `issue show` calls are needed.

**Suggestion:** Consider a `--full` flag on `graph deps` that includes the complete issue data for each node, similar to `jit query available --full`.

### 5. No epic completion summary
For epics, the most natural question is "what percentage is done?" The agent had to manually count states across dependencies. The `summary.by_state` field in `graph deps` helps, but it's flat (doesn't distinguish terminal vs non-terminal states).

**Suggestion:** Add a completion metric to the summary, e.g., `"completion": {"done": 10, "terminal": 11, "total": 13, "percent": 84}` where terminal includes done + rejected.

## Workflow That Would Be Ideal

An ideal single-call workflow for investigating an epic:

```
jit graph deps <id> --depth 0 --incomplete --full --json
```

This would return:
- Only non-terminal (incomplete) nodes in the tree
- Full issue details for each node (including their own sub-dependencies)
- A completion summary

This would reduce the 5-call investigation to 1 call.

## Conclusion

JIT is genuinely well-designed for AI agent use. The JSON-first approach, short IDs, and inline dependency data in `issue show` cover the common case well. The main optimization opportunity is reducing round-trips for "what's still blocking this?" queries on parent issues — a common agent workflow pattern.
