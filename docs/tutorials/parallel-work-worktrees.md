# Tutorial: Parallel Work with Git Worktrees

> **Diátaxis Type:** Tutorial  
> **Time to complete:** 15 minutes  
> **Prerequisites:** Basic familiarity with jit ([Quickstart](./quickstart.md))

This tutorial guides you through setting up parallel work using git worktrees. You'll learn how multiple agents can work on different issues simultaneously without conflicts.

## What You'll Learn

- Create a secondary worktree for parallel work
- Configure agent identity for coordination
- Claim issues to prevent conflicts
- Work in isolation and merge back

## Before You Start

You need:
- A git repository with jit initialized
- Basic understanding of git worktrees ([git-worktree docs](https://git-scm.com/docs/git-worktree))

## Step 1: Create a Secondary Worktree

First, create a new worktree from your main branch:

```bash
# From your main worktree
git worktree add ../my-feature -b feature/my-work
cd ../my-feature
```

Initialize jit in the new worktree:

```bash
jit init
```

Verify the worktree is detected:

```bash
jit worktree info
```

You should see output like:

```
Worktree Information:
  ID:         wt:abc12345
  Branch:     feature/my-work
  Root:       /path/to/my-feature
  Type:       secondary worktree
  Common dir: /path/to/main/.git
```

## Step 2: Configure Agent Identity

Agents need unique identities for claim coordination. Set yours:

```bash
# Option 1: Environment variable (recommended for sessions)
export JIT_AGENT_ID=agent:alice-feature

# Option 2: Persistent config file
mkdir -p ~/.config/jit
cat > ~/.config/jit/agent.toml << 'EOF'
[agent]
id = "agent:alice"
description = "Alice's development session"
EOF
```

Verify your identity:

```bash
echo $JIT_AGENT_ID
```

## Step 3: View Available Issues

From your secondary worktree, you can see all issues from the main worktree:

```bash
jit query available
```

This shows issues that are:
- Unassigned
- In "ready" state
- Not blocked by dependencies

The issue visibility works through a 3-tier fallback:
1. **Local `.jit/`** — Issues you've modified in this worktree
2. **Git HEAD** — Committed issues (canonical state)
3. **Main worktree `.jit/`** — Uncommitted issues from main

## Step 4: Claim an Issue

Before working on an issue, claim it to prevent conflicts:

```bash
# Find an available issue
jit query available

# Claim it (requires agent identity)
jit claim acquire <issue-id>
```

You should see:

```
✓ Acquired lease: abc123-def456...
  Issue: <issue-id>
  TTL: 600 seconds
```

The claim creates a **lease** that:
- Prevents other agents from claiming the same issue
- Expires after TTL (default: 10 minutes)
- Can be renewed if you need more time

View active claims:

```bash
jit claim list
```

## Step 5: Work on the Issue

Now work on your claimed issue. The key principle: **changes stay local until committed and merged**.

```bash
# Update issue state
jit issue update <issue-id> --state in_progress

# Do your work...
# (edit code, run tests, etc.)

# Run gates when ready
jit gate check-all <issue-id>

# Complete the issue
jit issue update <issue-id> --state done
```

Your changes to `.jit/` are isolated to this worktree until you commit them.

## Step 6: Commit and Merge

When your work is complete:

```bash
# Stage changes (including .jit/)
git add -A

# Commit
git commit -m "Complete feature work

Closes issue <issue-id>"

# Push and create PR (or merge directly)
git push origin feature/my-work
```

After merging to main, other worktrees will see your issue updates via git.

## Step 7: Clean Up

Release your claim (if not expired):

```bash
jit claim release <lease-id>
```

Remove the worktree when done:

```bash
cd ..
git worktree remove my-feature
```

## How It All Works Together

```
Main Worktree                    Secondary Worktree
─────────────                    ──────────────────
.jit/                            .jit/
├── issues/                      ├── issues/
│   └── task-1.json              │   └── (reads from main)
│                                │
.git/jit/  ◄────── Shared ──────►  (uses same control plane)
├── claims.jsonl                  
└── claims.index.json             
```

- **Issue data** is per-worktree (isolated)
- **Claims** are shared (via `.git/jit/`)
- **Visibility** spans all worktrees

## Try It Yourself

1. Create two worktrees
2. Set different `JIT_AGENT_ID` in each
3. Try claiming the same issue from both — the second should fail
4. Complete an issue in one worktree, commit, and see it update in the other

## Common Scenarios

### Scenario: Agent Times Out

If an agent crashes or the lease expires:

```bash
# List stale leases
jit claim list

# Force evict if needed (admin operation)
jit claim force-evict <lease-id> --reason "agent crashed"
```

### Scenario: Need More Time

Renew your lease before it expires:

```bash
jit claim renew <lease-id> --extension 600
```

### Scenario: Check Dependencies

Dependencies work across worktrees:

```bash
# See what blocks an issue (immediate deps)
jit graph deps <issue-id>

# See the full dependency tree
jit graph deps <issue-id> --depth 0
```

## What's Next?

- [How-to: Multi-Agent Coordination](../how-to/multi-agent-coordination.md) — Advanced coordination patterns
- [Configuration Reference](../reference/configuration.md) — Customize TTL, enforcement, and more
- [Troubleshooting Guide](../how-to/troubleshooting.md) — Common issues and solutions

## See Also

- [CLI Commands Reference](../reference/cli-commands.md) — Full command documentation
- Design document: `dev/design/worktree-parallel-work.md`
