# Worktree Dispatch Protocol

This protocol governs how the project-lead skill dispatches parallel sub-agent workers when the work needs file-system isolation (i.e., multiple workers editing the same crate / module / source tree).

It exists because two failure modes were observed in epic `babcf05e` session 6 that the Agent tool's built-in `isolation: "worktree"` parameter does not protect against:

1. **TRAP 1 — stale-base worktrees.** All four parallel workers' worktrees branched from a 41-commit-old ancestor instead of current `main` HEAD. The workers never saw the most recent Wave-3 closures. Their per-file diffs vs. main mixed real worker changes with apparent "reverts" that were just commits absent from the worker branch. Naïve integration would have clobbered the missing commits.
2. **TRAP 6 — worker file leakage into main.** Two of the four workers wrote files into the parent (main) checkout instead of their isolated worktree, contaminating main with worker artefacts that had to be detected and reverted post hoc.

See `dev/active/babcf05e-handoff-4.md` (in the gf2 project) for the full incident report.

## Protocol

The lead follows these four steps **every time** a parallel worktree dispatch is required.

### Step 1 — Pre-flight

```bash
git checkout main
git status        # must be clean (no staged or unstaged changes)
```

If main is dirty, commit, stash, or revert first. Dispatch from a dirty `main` is forbidden — the leak-check guard in step 4 cannot work without a clean baseline.

### Step 2 — Manual worktree creation + snapshot

Run the project's wrapper script:

```bash
scripts/dispatch-worker-worktree.sh <short-id-1> <short-id-2> ...
```

The script lives in this skill's directory and is generic across projects (it operates on whatever git repo your current working directory belongs to). It:

1. Verifies the current branch is `main` and the working tree is clean.
2. Snapshots `git status -uall --porcelain` of main to `/tmp/lead-pre-dispatch-<ts>.txt` and links `/tmp/lead-pre-dispatch-latest.txt` to it.
3. For each short-id, runs `git worktree add -b worktree-agent-<short-id> .claude/worktrees/agent-<short-id> <main-HEAD-sha>`. Anchoring explicitly on `main`'s commit SHA (not the branch name) prevents any race where main moves mid-dispatch.
4. Verifies each worktree's HEAD SHA exactly matches `main`'s HEAD SHA.
5. Emits a prompt-header block per dispatched issue.

### Step 3 — Dispatch

Call `Agent` with:

- `subagent_type: "general-purpose"`
- **No `isolation` parameter.** Manual worktree creation in step 2 replaces it. Using `isolation: "worktree"` after manual creation would create a *second*, stale-base worktree alongside the manual one and re-introduce TRAP 1.
- The prompt prefixed with the boilerplate emitted by the dispatch script. The boilerplate names the worktree path, the branch, and the path-discipline rules (no `/home/...` absolute paths, run every command from the worktree root).
- `run_in_background: true` for parallel work. Background mode is required so the lead can dispatch the rest of the wave without waiting on each worker.

### Step 4 — Post-completion leak check

After all dispatched workers complete (or fail / time out), run:

```bash
scripts/check-leak-into-main.sh
```

This compares main's current `git status -uall` against the dispatch-time snapshot. Files unique to the current state are likely worker leaks. The script prints recovery commands.

If leaks are detected:

- **Tracked-file modifications** (` M <path>`): `git restore <path>` to revert.
- **Untracked files** (`?? <path>`): `mv` them to the correct worktree (and amend the worker's preserve commit if applicable), or `rm` if unwanted.

Re-run the leak check until it returns clean before doing any commits on main.

## Lead-preserve workflow on worker truncation

If a worker terminates abnormally (API outage, timeout, OOM) leaving uncommitted changes in its worktree, capture them on the worker branch so the next-session lead can review:

```bash
cd .claude/worktrees/agent-<short-id>
git add -A
git -c user.name="<your-name> (lead-preserve)" commit -m \
    "wip(jit:<short-id>): preserve session-N worker WIP after <reason>"
```

Lead-preserve commits are **never merged to main**. They live on the worker branch as a salvage point. The next-session lead decides whether to rebase the branch onto current main, fold into a rework cycle, or discard.

## Recovery — when this protocol was not followed

If you discover after the fact that worktrees were created via Agent's `isolation: "worktree"` and may have a stale base:

```bash
# For each worker branch, rebase onto current main:
cd .claude/worktrees/agent-<id>
git merge-base HEAD main          # inspect ancestor
git log --oneline ^HEAD main      # commits on main not on the worker branch
git rebase main                    # bring the worker up to current main
# resolve conflicts; re-run cargo-ci before any integration to main
```

After rebase, re-evaluate the worker's diff vs. main — pre-rebase, the diff was contaminated with apparent "reverts" of commits that simply weren't on the worker branch. Post-rebase, the diff is the worker's actual work.

Document the rebase in the next handoff. The worker's spec compliance must be re-checked: a worker that didn't see the existing fp_generic / dispatch-hoist / etc. on main may have accidentally duplicated or collided with that code.

## Why each guard works

| Failure mode | Guard | Where it fires |
|---|---|---|
| Worktree branched from stale ancestor | Step 2 verifies `git rev-parse HEAD` of every worktree equals `main`'s | Pre-dispatch — a mismatch fails the script |
| Worker writes files into main's checkout | Step 4 diffs `git status -uall` vs. snapshot | Post-dispatch — a leak shows up as "entries present now, not in snapshot" |
| Lead forgets the post-dispatch check | The dispatch script's final line reminds the lead | Lead reads the reminder every time |
| Worker bundles commits, then crashes mid-batch | Per-issue dispatch prompt rule: "commit each spiral step before proceeding" | Worker prompt — this is on the worker, not on the protocol |

The third row is worth emphasising: the post-dispatch check is the only guard against TRAP 6, and it is one shell line. The lead must run it after every wave, even when the wave looks clean by eyeball. Eyeballing is what missed the leak in babcf05e session 6 in the first place.

## Implementation

The scripts live in this skill's `scripts/` subdirectory and are project-agnostic. They use `git rev-parse --show-toplevel` from the agent's current working directory to anchor onto whatever repo the lead is operating in, so the same scripts work for every project that uses this skill.

- `scripts/dispatch-worker-worktree.sh`
- `scripts/check-leak-into-main.sh`

Read them directly if you need to understand the invariants. Both are shellcheck-clean and smoke-tested. They take no project-specific configuration.

Do **not** copy the scripts into individual project trees — the canonical copy lives with the skill, and projects should reference them via the relative path conventions used in this protocol so improvements propagate to every project automatically.
