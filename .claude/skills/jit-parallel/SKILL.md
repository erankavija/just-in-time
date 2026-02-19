---
name: jit-parallel
description: Orchestrate parallel implementation of multiple independent JIT issues using sub-agents. Use when asked to implement, work on, or dispatch several issues simultaneously. Handles pre-flight conflict analysis, sub-agent dispatch, result verification, and commit.
compatibility: Designed for Claude Code with JIT MCP tools available
---

# Parallel JIT Issue Orchestration

Dispatch multiple independent issues to sub-agents running concurrently in the same worktree. All agents share the filesystem, so conflict-free file selection is essential.

## Step 1: Pre-flight — select and validate issues

1. For each candidate issue, run `mcp__jit__jit_issue_show` to get its full description and any linked design docs.
2. Confirm all issues are in `ready` state with no unresolved dependencies (`mcp__jit__jit_graph_deps`).
3. **Assess file overlap** using the heuristics in [references/conflict-heuristics.md](references/conflict-heuristics.md). Issues that touch the same file must be serialised, not parallelised.
4. Claim all implementation issues before dispatching (`mcp__jit__jit_issue_claim`, assignee `agent:claude`). Review-only issues do not need claiming.

## Step 2: Dispatch sub-agents

Send a **single message** with one `Task` tool call per issue so they run concurrently.

Use agent type `general-purpose` for implementation tasks, `Explore` for read-only research.

For each sub-agent, compose a prompt using the template in [references/agent-prompt-template.md](references/agent-prompt-template.md). Key fields to fill in:
- Issue ID, title, full description
- Acceptance criteria or success criteria from the issue
- Any linked design doc paths
- Whether the task is implementation or review

## Step 3: Post-flight — verify and integrate

After all agents return:

1. **Review every diff** — read changed files, check for unintended edits (e.g. a `cargo fmt` reflow touching files from another agent's work).
2. **Run the full suite:**
   ```bash
   cargo test --workspace --quiet
   cargo clippy --workspace --all-targets
   cargo fmt --all -- --check
   ```
3. Fix any issues introduced by agent interactions (usually trivial formatting conflicts).
4. Mark each issue done: `mcp__jit__jit_issue_update` with `state=done`.
5. Commit using the JIT convention (see below).

## Commit convention

```
jit: <short summary of combined work>

- <issue title 1> (<short-id>)
- <issue title 2> (<short-id>)

jit: <short-id-1> <short-id-2>

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
```

Always stage `.jit/events.jsonl` and all modified `.jit/issues/*.json` files alongside code changes.

## When to serialise instead

- Two issues modify the same source file
- One issue's output is input to the other (dependency)
- Either issue modifies `domain/types.rs`, `main.rs`, or shared test infrastructure — high-traffic files warrant extra caution
- Either issue adds a new `Event` variant (match exhaustiveness forces coordinated edits)

## Worktree-based parallelism (larger batches)

For 4+ issues or issues with unavoidable file overlap, use git worktrees instead. See [references/worktree-mode.md](references/worktree-mode.md). The JIT claim system spans worktrees automatically via `.git/jit/`.
