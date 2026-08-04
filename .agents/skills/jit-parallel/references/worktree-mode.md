# Worktree-Based Parallelism

Use when running 4+ agents or when issues have unavoidable file overlap.

## Setup: use the dispatch script

The canonical procedure is `jit-execution-lead/references/worktree-dispatch-protocol.md`.
Read it before dispatching. Do not hand-roll `git worktree add`, and do not use the
Agent tool's `isolation: "worktree"` parameter: both skip the guards the script provides.

```bash
.agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh <short-id-1> <short-id-2> ...
```

Per issue, the script creates `.agents/worktrees/agent-<short-id>` on branch
`worktree-agent-<short-id>`, anchored to `main`'s HEAD **SHA** rather than the branch
name, then verifies each worktree's HEAD matches. It also snapshots `git status -uall`
so the post-wave leak check has a baseline. Two guards, two documented incidents:
worktrees branched from a stale ancestor, and workers writing into the parent checkout.

`jit init` has no place here. `.jit/` is tracked in git, so every worktree already
carries the repository's issues, config, and gate registry. Running `jit init` inside
a worktree re-scaffolds state that already exists.

## Dispatch agents

Prefix each agent's prompt with the header block the script emits. It names the
worktree path, the branch, and the path-discipline rules. Never paste an absolute
path from another machine or another session; take the path from the script's output.

Dispatch with `subagent_type: "general-purpose"` and **no** `isolation` parameter.

Claims are shared via `.git/jit/` (one control plane across all worktrees), so agents
cannot double-claim the same issue. The `active_claims` column in `jit worktree list`
counts active *leases* (from `jit claim acquire`), not *assignments* (from
`jit issue claim`). Assignments live in `.jit/issues/` and do not affect this count.

## Worker completion is a commit, not an idle ping

An agent going idle proves nothing: it may have finished and forgotten to commit, or
still be mid-edit. Before treating a worker as done, inspect its worktree:

```bash
git -C .agents/worktrees/agent-<short-id> log --oneline <base-sha>..HEAD
git -C .agents/worktrees/agent-<short-id> status --porcelain   # must be empty
```

A clean tree with no commits means the worker produced nothing. A dirty tree means it
is still working, or it stopped without committing. Ask the worker before re-dispatching;
never hard-kill a background agent holding uncommitted work.

## Merge

After each agent commits to its branch, merge sequentially into `main`:

```bash
git merge --no-ff worktree-agent-<short-id>
```

`.jit/events.jsonl` is declared `merge=union` in `.gitattributes`, so git concatenates
both sides of that per-worktree append-only log without a conflict. The claim log
(`.git/jit/claims.jsonl`) needs no merge driver: it lives in the shared control plane
outside the versioned `.jit/` tree, so every worktree already reads and writes the same
physical file. Code conflicts require manual resolution.

After **each** merge — before the next merge and before any further commit lands on top —
run the project's build-and-test gate on the result:

```bash
git status --porcelain    # must be empty: what the gate judges is then the merge commit's tree
scripts/cargo-ci.sh       # this project's gate; use whichever one your project configures
```

A textually clean merge can otherwise leave the mainline broken: a worker branch anchored
before a module deletion, merged after it, re-adds a declaration referencing a file that no
longer exists; a branch that changes a function's signature merges cleanly with a branch that
adds a caller of the old form. Neither merge sees a textual overlap, so neither conflicts.
The per-issue gates and the leak check below both evidence a pre-merge working tree, so they
stay green while the merged mainline is broken; a gate run on the merged tree is what catches
it. That gate must compile **and run** the tests — a build-only check compiles neither test
targets nor dev-dependencies, so it passes a merge that breaks only test code. Its verdict is
the merge's verdict only while the tree is clean, and gating once after several merges judges
the final tree alone. On failure, amend or fix-forward the merge commit and re-run before
merging the next branch.

After the wave completes, run the leak check before committing anything on `main`:

```bash
.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh
```

It compares `main`'s working tree against the pre-dispatch snapshot. Files present now
and absent from the snapshot are candidate worker leaks. Note that a lead which
evaluates gates on `main` legitimately writes `.jit/gate-runs/`, `.jit/events.jsonl`,
and issue records, so those appear too: attribute each entry before reverting it.

## Cleanup

Worktrees and branches accumulate across sessions. When a wave closes, verify each
branch is merged, then remove it:

```bash
git merge-base --is-ancestor worktree-agent-<short-id> main \
  && git worktree remove .agents/worktrees/agent-<short-id> \
  && git branch -d worktree-agent-<short-id>
```

Remove only worktrees you created. A branch that is an ancestor of `main` may still be
the working directory of a live agent in another session.

## When worktrees are worth the overhead

- 4+ concurrent agents
- Issues that both modify the same high-traffic file (unavoidable overlap)
- Long-running tasks where you want agents to commit intermediate progress
- When you want each agent's work independently reviewable as a branch
