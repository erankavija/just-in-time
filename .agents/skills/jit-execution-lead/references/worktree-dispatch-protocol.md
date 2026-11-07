# Worktree Dispatch Protocol

This protocol governs how the jit-execution-lead skill dispatches parallel sub-agent workers when the work needs file-system isolation (i.e., multiple workers editing the same crate / module / source tree). It guards against two failures that the Agent tool's `isolation: "worktree"` parameter does not prevent: worktrees branched from a stale ancestor, and workers writing into the main checkout.

## Protocol

The lead follows these steps **every time** a parallel worktree dispatch is required.

### Step 1 — Pre-flight

```bash
git checkout main
git status        # tracked changes allowed only under the resolved data root
```

The script obtains the absolute data root from `jit worktree info --field data_root` (requires `jit`). Staged or unstaged changes under that root may remain: workers branch from the committed main revision. Any other tracked change blocks dispatch and is named in the refusal; commit or stash it first. An external data root exempts no checkout path.

### Step 2 — Create worktrees

```bash
.agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh <short-id-1> <short-id-2> ...
```

Per short-id, the script creates `.agents/worktrees/agent-<short-id>` on branch `worktree-agent-<short-id>`, anchored to `main`'s HEAD SHA and verified against it. It snapshots main's `git status -uall` for the leak check, seeds build caches from the shared cache pool, creates a per-worker temporary directory the worker exports as `TMPDIR`, and emits a prompt-header block per issue. Its header comment documents the environment variables (`LEAD_CACHE_POOL`, `LEAD_WORKTREE_TMP_ROOT`, `LEAD_SNAPSHOT_DIR`); use the same values for dispatch and reclaim.

### Step 3 — Dispatch

Call `Agent` with:

- **No `isolation` parameter.** Using `isolation: "worktree"` after manual creation creates a second, stale-base worktree.
- The prompt prefixed with the header block the script emitted. It names the worktree path, the branch, and the path-discipline rules.
- `run_in_background: true` for parallel work.

Worker worktrees are linked checkouts, so jit's `worktree.write_policy` (refusing by default) governs state-mutating jit commands run from them: workers use the tracker read-only, and the lead owns issue state from the primary.

A worker whose sandbox cannot write outside its worktree (observed with codex linked-checkout workspaces) must leave its toolchain's build-output and cache directories at their in-worktree defaults; dispatch-time seeding still reaches it.

### Step 4 — Post-completion leak check

After all dispatched workers complete (or fail / time out), run:

```bash
.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh
```

It compares main's current `git status -uall` against the dispatch-time snapshot and prints recovery commands for each leak:

- **Tracked-file modifications** (` M <path>`): `git restore <path>` to revert.
- **Untracked files** (`?? <path>`): `mv` them to the correct worktree (and amend the worker's preserve commit if applicable), or `rm` if unwanted.

Re-run the leak check until it returns clean before doing any commits on main. Run it after every wave, even when the wave looks clean.

### Step 5 — Gate the merged tree

Merge each worker branch into `main` sequentially, and after **each** merge — before the next merge and before any further commit lands on top — run the project's build-and-test gate on the result:

```bash
.agents/skills/jit-execution-lead/scripts/land.sh worktree-agent-<short-id>
git status --porcelain    # must be empty: what the gate judges below is then the merge commit's tree
scripts/cargo-ci.sh       # this project's gate; use whichever one your project configures
```

`land.sh` holds the repository write lock while it commits pending tracker state as `chore(jit): sync tracker` and merges. On a conflict it aborts the merge, keeps that commit, and names the paths: resolve them with `git merge` by hand.

The gate must compile **and run** the project's tests; a build-only check is insufficient. Its verdict belongs to the merge commit only when the tree is clean, and gating once after several merges judges only the final tree.

If the gate reports failure, repair the merge commit (`git commit --amend` or a follow-up fix commit) and re-run it before merging the next branch.

When the check runs through the tracker's own gate evaluation (`jit gate evaluate`), another agent's active lease on the issue refuses it.

### Step 6 — Reclaim worktrees once the wave closes

Merging a branch does not reclaim its worktree. After each wave closes, with no build running:

```bash
.agents/skills/jit-execution-lead/scripts/reclaim-worker-worktree.sh <short-id>...
```

The script harvests each worktree's build caches into the shared cache pool, removes the worktree and its temporary directory, and prunes. It refuses a worktree whose branch has commits unmerged into main; merge first, or set `LEAD_RECLAIM_FORCE=1` for an abandoned worktree. Removing a worktree keeps its branch, so lead-preserve salvage points survive. Use the script rather than a bare `git worktree remove`, which discards the warm caches.

The cache pool is a cache, never a source of truth: delete it when it grows too large and the next harvest rebuilds it.

## Lead-preserve workflow on worker truncation

If a worker terminates abnormally (API outage, timeout, OOM) leaving uncommitted changes in its worktree, capture them on the worker branch so the next-session lead can review:

```bash
cd .agents/worktrees/agent-<short-id>
git add -A
git -c user.name="<your-name> (lead-preserve)" commit -m \
    "wip(jit:<short-id>): preserve session-N worker WIP after <reason>"
```

Lead-preserve commits are **never merged to main**. They live on the worker branch as a salvage point. The next-session lead decides whether to rebase the branch onto current main, fold into a rework cycle, or discard.

## Implementation

The scripts live in this skill's `scripts/` subdirectory and are project-agnostic: they anchor on `git rev-parse --show-toplevel` of the current working directory. Each script's header comment documents its environment variables and exit codes. Reference them by path from this skill; do not copy them into project trees.

- `.agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh`
- `.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh`
- `.agents/skills/jit-execution-lead/scripts/reclaim-worker-worktree.sh`
- `.agents/skills/jit-execution-lead/scripts/land.sh`
