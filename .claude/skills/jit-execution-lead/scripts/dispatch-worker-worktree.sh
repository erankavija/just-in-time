#!/usr/bin/env bash
# Project-lead utility: create per-issue worktrees anchored to current
# main HEAD and snapshot main's working tree so post-dispatch leak
# checks can detect files a worker accidentally writes into main
# instead of its own worktree.
#
# Replaces Agent's `isolation: "worktree"` parameter which has been
# observed to branch from a stale ancestor instead of current main
# HEAD. Operates on whatever git repo the current working directory
# belongs to — invoke from anywhere inside the target repo.
#
# Reference: references/worktree-dispatch-protocol.md (in this skill).
#
# Usage:
#   scripts/dispatch-worker-worktree.sh <short-id> [<short-id>...]
#
# Effects:
#   - Snapshots `git status -uall --porcelain` of main to
#     /tmp/lead-pre-dispatch-<ts>.txt and symlinks
#     /tmp/lead-pre-dispatch-latest.txt to it.
#   - Creates one worktree per short-id at
#     .claude/worktrees/agent-<short-id> on branch
#     worktree-agent-<short-id> anchored to current main HEAD.
#   - Verifies every worktree HEAD == main HEAD.
#   - Prints a prompt-header block to paste at the top of every
#     dispatched Agent prompt.
#
# Exit codes:
#   0 — all worktrees created and verified
#   1 — pre-flight failed (not on main, dirty tree, branch collision,
#       worktree HEAD mismatch)
#   2 — bad invocation

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "usage: $0 <short-id> [<short-id>...]" >&2
    exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

current_branch="$(git symbolic-ref --short HEAD 2>/dev/null || true)"
if [[ "$current_branch" != "main" ]]; then
    echo "ERROR: HEAD is not 'main' (currently '$current_branch'). Dispatch from main." >&2
    exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "ERROR: main has uncommitted changes. Commit or stash before dispatching." >&2
    git status --short >&2
    exit 1
fi

main_sha="$(git rev-parse HEAD)"
short_main_sha="$(git rev-parse --short HEAD)"
echo "[ok] main at ${short_main_sha} (${main_sha}); working tree clean"

ts="$(date -u +%Y%m%d-%H%M%S)"
snapshot="/tmp/lead-pre-dispatch-${ts}.txt"
git status -uall --porcelain > "$snapshot"
ln -sfn "$snapshot" /tmp/lead-pre-dispatch-latest.txt
snapshot_lines="$(wc -l < "$snapshot")"
echo "[ok] snapshot: $snapshot (${snapshot_lines} entries)"
echo "     latest -> /tmp/lead-pre-dispatch-latest.txt"

mkdir -p .claude/worktrees
created=()
for sid in "$@"; do
    wt_path=".claude/worktrees/agent-${sid}"
    wt_branch="worktree-agent-${sid}"

    if [[ -e "$wt_path" ]]; then
        echo "ERROR: $wt_path already exists. Remove it before dispatching $sid." >&2
        exit 1
    fi

    if git rev-parse --verify --quiet "refs/heads/${wt_branch}" > /dev/null; then
        echo "ERROR: branch ${wt_branch} already exists. Delete it before dispatching $sid." >&2
        exit 1
    fi

    git worktree add -b "$wt_branch" "$wt_path" "$main_sha" > /dev/null
    wt_head="$(git -C "$wt_path" rev-parse HEAD)"

    if [[ "$wt_head" != "$main_sha" ]]; then
        echo "ERROR: $wt_path HEAD ($wt_head) != main HEAD ($main_sha)" >&2
        exit 1
    fi

    echo "[ok] $wt_path  branch=$wt_branch  base=${short_main_sha}"
    created+=("$sid")
done

cat <<EOF

==== Agent prompt header (paste at top of every dispatched prompt) ====
EOF

for sid in "${created[@]}"; do
    cat <<EOF

---- prompt header for $sid ----
You are dispatched to JIT issue $sid. Your worktree is at:
  $repo_root/.claude/worktrees/agent-$sid
on branch worktree-agent-$sid, anchored to main at ${main_sha}.

Hard rules for path discipline (worktree-dispatch-protocol):
- Run every shell command from your worktree root. Prefix tool calls
  with 'cd $repo_root/.claude/worktrees/agent-$sid && ...' if your
  shell state has drifted.
- Use only paths relative to the worktree root in your tool calls.
  Absolute paths starting with /home/ are forbidden — they leak
  files into main's checkout.
- Never run 'git checkout', 'git switch', 'git worktree add/remove'.
- Commit on your worktree branch only. Do not push.
EOF
done

echo
echo "Post-completion: run scripts/check-leak-into-main.sh (this skill's scripts/)"
echo "to verify main's working tree did not absorb any worker output."
