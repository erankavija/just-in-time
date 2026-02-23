# AI Tool Worktree Compatibility Study

**Date**: 2026-02-23
**Status**: Research complete — actionable next steps identified
**Goal**: Understand how Claude Code and Cursor implement parallel work with git worktrees, identify where JIT fits, surface design tensions, and recommend concrete steps toward tool-agnosticism.

---

## Summary

Both Claude Code and Cursor use git worktrees as the isolation primitive for parallel agents. Neither tool provides a work coordination layer — no dependency tracking, no claim system, no queue. JIT fills that vacuum and is highly complementary to both. The primary integration work needed is: normalization of the worktree creation step, tool-specific setup hook snippets, and environment-based identity auto-detection.

---

## Claude Code Worktree Model

Claude Code (Anthropic CLI) provides native worktree support via `--worktree <name>`:

- Creates `.claude/worktrees/<name>/` **inside the repository**, branching as `worktree-<name>` from the default remote
- Automatic lifecycle: no changes → auto-remove on exit; uncommitted changes → prompt user
- Subagents support `isolation: worktree` frontmatter — each subagent gets its own worktree, automatically cleaned up
- Sessions persist per worktree; `/resume` shows sessions across all worktrees with branch info
- Non-git VCS supported via `WorktreeCreate`/`WorktreeRemove` hooks

**Coordination**: none built-in. Claude Code isolates the *environment* but does not track what each session is doing or prevent two sessions from working the same logical task.

---

## Cursor Worktree Models

Cursor 2.0 (Oct 2025) ships two distinct parallel agent modes:

### Mode 1: Local Parallel Agents (git worktrees)

- Each agent runs in its own git worktree with an isolated working directory on the same machine
- Configured via `.cursor/worktrees.json` — setup commands run when a worktree is created:
  ```json
  {
    "setup-worktree": [
      "npm ci",
      "cp $ROOT_WORKTREE_PATH/.env .env"
    ]
  }
  ```
- `$ROOT_WORKTREE_PATH` env var references the primary repo during setup
- Max 20 worktrees per workspace, LRU auto-cleanup
- LSP/linting unavailable in worktrees (current limitation)
- Merge back via "Apply" button — full overwrite or native conflict resolution

### Mode 2: Cloud Agents (formerly Background Agents)

- Isolated Ubuntu VMs on AWS; fully remote
- Clone repo from GitHub/GitLab, push results to a new branch
- Configured via `.cursor/environment.json` (install commands, named terminal processes, snapshots)
- Async — results are reviewed and merged after the agent finishes
- Requires GitHub/GitLab integration with read-write access

**Coordination**: none built-in. Cursor's parallel model is either competitive (Best-of-N: same prompt across multiple models, pick best result) or independently assigned by the human.

---

## Three-Way Comparison

| Dimension | Claude Code | Cursor Local | Cursor Cloud |
|---|---|---|---|
| Worktree location | `.claude/worktrees/<name>/` (inside repo) | Sibling directories (outside repo) | Remote VM |
| Branch naming | `worktree-<name>` | User/auto-defined | New branch per run |
| Setup hook | None built-in | `.cursor/worktrees.json` | `.cursor/environment.json` |
| Subagents | `isolation: worktree` frontmatter | `.cursor/agents/` config files | N/A |
| Session resume | `--resume`, session picker | Within conversation history | Async branch review |
| Work coordination | None | None | None |
| Dependency tracking | None | None | None |
| JIT integration | Manual (`jit init`, `JIT_AGENT_ID`) | Manual + `.cursor/worktrees.json` hook | Would need env.json step |

**JIT fills the coordination vacuum that all three lack.**

---

## Critical Design Conflict: Worktree Placement

The JIT architecture design (`dev/design/worktree-parallel-work.md`) explicitly states:

> "Worktrees must be placed **outside** the main repository directory. Placing them inside (e.g., `project-root/worktrees/`) causes agents to see each other's files during searches and glob operations, breaking isolation."

Claude Code defaults to `.claude/worktrees/<name>/` — **inside** the repository. This creates a real isolation risk: an agent searching `**/*.rs` from `.claude/worktrees/feature-a/` would traverse into `.claude/worktrees/feature-b/` and see another agent's in-progress files.

JIT's own coordination is unaffected — it uses `.git/jit/` as the shared control plane, which is correctly accessed via the shared git directory regardless of worktree placement. The problem is AI tool search leakage between sibling worktrees in the nested case.

**Cursor's approach** (sibling directories: `../repo-feature-a/`) naturally avoids this problem. It matches JIT's design recommendation.

**Mitigations for the inside-repo case**:
1. Add `.claude/worktrees/` to `.gitignore` (CC already recommends this; prevents git noise but not search leakage)
2. Each tool's AI search scope is contained to the worktree root in practice, but not guaranteed
3. A supported `--inside` flag in `jit worktree create` could document the tradeoff explicitly

This is the concrete argument for documenting sibling-directory worktrees as JIT's preferred pattern, with an acknowledged CC-compat mode.

---

## How JIT and the AI Tools Compose

The ideal three-level stack:

```
┌──────────────────────────────────────────────────────┐
│  Orchestrator (main session, main branch)             │
│  • jit query available  → identify ready work         │
│  • claude --worktree <task> per issue                 │
│  • jit claim list       → monitor all parallel work   │
│  • merge worktrees when done                          │
└────────────────┬─────────────────────────────────────┘
                 │ dispatches
      ┌──────────┼──────────┐
      ▼          ▼          ▼
  ┌────────┐ ┌────────┐ ┌────────┐
  │Worktree│ │Worktree│ │Worktree│
  │agent:A │ │agent:B │ │agent:C │
  │                               │
  │  export JIT_AGENT_ID=agent:X  │
  │  jit claim acquire <id>       │
  │  ...work...                   │
  │  jit gate check-all           │
  │  git commit && push           │
  │  jit claim release            │
  └────────┘ └────────┘ └────────┘
```

Claude Code provides the **environment** (isolated directories, lifecycle). Cursor provides two environment options (local or cloud). JIT provides the **coordination layer** (who works on what, in what order, with what constraints).

---

## Proposed Next Steps

### 1. Add `jit worktree create` command

A single command normalizing worktree creation across tools:

```bash
jit worktree create <name> [-b <branch>] [--outside | --inside]
```

- `--outside` (default): `../repo-name-<name>/` — full isolation, Cursor native, JIT recommended
- `--inside`: `.claude/worktrees/<name>/` — Claude Code compat; documents the search-leakage tradeoff in the output

Handles: `git worktree add`, `jit init`, `worktree.json` creation, `.gitignore` entry.

### 2. Document `.cursor/worktrees.json` integration

A copyable snippet for Cursor users to bootstrap JIT in every new worktree automatically:

```json
{
  "setup-worktree": [
    "jit init --if-not-exists",
    "echo 'export JIT_AGENT_ID=agent:cursor-$(basename $PWD)' >> ~/.bashrc"
  ]
}
```

This gives Cursor users zero-friction JIT initialization with no manual steps.

### 3. Document `.cursor/environment.json` template for Cloud Agents

```json
{
  "install": ["cargo install --git https://github.com/... jit", "jit init --if-not-exists"],
  "terminals": {
    "agent": "export JIT_AGENT_ID=agent:cursor-cloud && jit issue claim-next $JIT_AGENT_ID"
  }
}
```

Documents the async cloud agent pattern; aspirational for now, but sets the direction.

### 4. Environment auto-detection in JIT

Reduce manual identity configuration by detecting the running tool:

| Env var / signal | Inferred tool | Auto agent ID pattern | Worktree mode default |
|---|---|---|---|
| `CURSOR_WORKSPACE_ID` | Cursor | `agent:cursor-<short-hash>` | outside |
| `.claude/` in repo root | Claude Code | `agent:claude-<worktree-name>` | inside (compat) |
| `GITHUB_ACTIONS` | CI | `ci:github-actions-<run-id>` | off |
| none | Unknown | prompt | off |

Implementation: `[autodetect]` section in `~/.config/jit/agent.toml` and/or startup detection in the JIT binary.

### 5. Add a `AGENTS.md` bootstrap convention

A per-repo neutral file (analogous to `CLAUDE.md`) that any AI tool reads when starting a new agent session. Tool-specific reading:

- Claude Code: mention in `CLAUDE.md` with a pointer
- Cursor: add to `.cursor/rules/`
- Others: include in repo README

Content template:
```markdown
# Agent Bootstrap

When starting a new parallel session in this repo:
1. Create a worktree: `jit worktree create <your-task-name>`
2. Set identity: `export JIT_AGENT_ID=agent:<your-name>`
3. Find available work: `jit query available`
4. Claim a task: `jit issue claim-next $JIT_AGENT_ID`
5. Check dependencies before starting: `jit graph deps <issue-id>`
```

### 6. Update JIT architecture doc and tutorial for dual worktree patterns

`dev/design/worktree-parallel-work.md` and `docs/tutorials/parallel-work-worktrees.md` should:
- Acknowledge Claude Code's `.claude/worktrees/` convention explicitly
- Document the inside-repo isolation tradeoff
- Show tool-specific setup sections (CC, Cursor local, Cursor cloud)
- Align on sibling-directory as the recommended default with a clearly labeled CC-compat note

---

## Sources

- [Parallel Agents | Cursor Docs](https://cursor.com/docs/configuration/worktrees)
- [Cloud Agents | Cursor Docs](https://cursor.com/docs/background-agent)
- [Cursor 2.0 Blog](https://cursor.com/blog/2-0)
- [Agent Best Practices | Cursor](https://cursor.com/blog/agent-best-practices)
- [Subagents | Cursor Docs](https://cursor.com/docs/context/subagents)
- [Common Workflows | Claude Code Docs](https://code.claude.com/docs/en/common-workflows)
- `dev/design/worktree-parallel-work.md` — JIT architecture design
- `docs/tutorials/parallel-work-worktrees.md` — JIT tutorial
- `docs/how-to/multi-agent-coordination.md` — JIT coordination how-to
- `dev/experiments/worktree-manual-coordination-experiment.md` — original parallel work experiment
