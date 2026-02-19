# Worktree-Based Parallelism

Use when running 4+ agents or when issues have unavoidable file overlap.

## Setup (once per batch)

```bash
# Create one worktree per issue
git worktree add ../jit-wt-1 -b agents/issue-1
git worktree add ../jit-wt-2 -b agents/issue-2

# Initialise JIT in each
(cd ../jit-wt-1 && jit init)
(cd ../jit-wt-2 && jit init)
```

## Dispatch agents

Pass each agent its worktree path and a unique agent ID:

```
You are working in worktree /home/vkaskivuo/Projects/jit-wt-1.
Set JIT_AGENT_ID=agent:claude-wt-1 for all jit commands.
...
Commit your changes at the end: git -C /path/to/wt add -A && git -C /path/to/wt commit -m "..."
```

Claims are shared via `.git/jit/` — agents cannot double-claim the same issue.

## Merge

After all agents commit to their branches, merge sequentially:

```bash
git checkout main
git merge agents/issue-1   # resolve any conflicts
git merge agents/issue-2
git worktree remove ../jit-wt-1
git worktree remove ../jit-wt-2
```

The JIT merge driver handles `events.jsonl` automatically. Code conflicts require manual resolution.

## When worktrees are worth the overhead

- 4+ concurrent agents
- Issues that both modify `main.rs` or `types.rs` (unavoidable overlap)
- Long-running tasks where you want agents to commit intermediate progress
- When you want each agent's work independently reviewable as a branch
