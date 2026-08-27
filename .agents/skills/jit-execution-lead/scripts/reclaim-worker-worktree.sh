#!/usr/bin/env bash
# jit-execution-lead utility: reclaim per-issue worker worktrees after a wave
# closes, harvesting their build/dependency caches into the shared cache pool
# so the next wave starts warm. Counterpart of dispatch-worker-worktree.sh,
# which seeds each new worktree from that pool.
#
# Toolchain-agnostic: harvests every top-level directory the worktree's git
# ignores (build outputs and caches are ignored by definition) into
# "$pool/<name>", newest file wins, or exactly the space-separated names in
# LEAD_CACHE_DIRS. Pool: LEAD_CACHE_POOL, default <repo>/.agents/cache-pool.
#
# Refuses a worktree whose branch has commits unmerged into main (those carry
# salvage points); LEAD_RECLAIM_FORCE=1 overrides for an abandoned worktree.
#
# Reference: references/worktree-dispatch-protocol.md (in this skill).
#
# Usage:
#   .agents/skills/jit-execution-lead/scripts/reclaim-worker-worktree.sh <short-id> [<short-id>...]
#
# Exit codes:
#   0 — all named worktrees reclaimed (cache harvested, worktree removed)
#   1 — a worktree was skipped (unmerged commits) or removal failed
#   2 — bad invocation

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "usage: $0 <short-id> [<short-id>...]" >&2
    exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

pool="${LEAD_CACHE_POOL:-$repo_root/.agents/cache-pool}"
status=0

# Cache directories for one worktree: LEAD_CACHE_DIRS when set, else every
# top-level directory git ignores in that worktree.
cache_dirs_for() {
    local wt="$1" entry
    if [[ -n "${LEAD_CACHE_DIRS:-}" ]]; then
        echo "$LEAD_CACHE_DIRS"
        return
    fi
    for entry in "$wt"/*/; do
        [[ -d "$entry" && ! -L "${entry%/}" ]] && basename "$entry"
    done | git -C "$wt" check-ignore --stdin 2>/dev/null || true
}

for sid in "$@"; do
    wt_path=".agents/worktrees/agent-${sid}"
    wt_branch="worktree-agent-${sid}"

    if [[ ! -d "$wt_path" ]]; then
        echo "[skip] $wt_path does not exist" >&2
        status=1
        continue
    fi

    if [[ "${LEAD_RECLAIM_FORCE:-0}" != "1" ]] \
        && git rev-parse --verify --quiet "refs/heads/${wt_branch}" > /dev/null \
        && [[ -n "$(git log --oneline "main..${wt_branch}" -- 2>/dev/null)" ]]; then
        echo "[skip] ${wt_branch} has commits unmerged into main; not removing." >&2
        echo "       Merge it, or set LEAD_RECLAIM_FORCE=1 for an abandoned worktree." >&2
        status=1
        continue
    fi

    for name in $(cache_dirs_for "$wt_path"); do
        src="$wt_path/$name"
        [[ -d "$src" && ! -L "$src" ]] || continue
        mkdir -p "$pool/$name"
        # Newest file wins so a stale harvest never clobbers fresher artifacts.
        if command -v rsync > /dev/null; then
            rsync -a --update "$src/" "$pool/$name/"
        else
            cp -au "$src/." "$pool/$name/"
        fi
        echo "[ok] harvested $src -> $pool/$name"
    done

    git worktree remove --force "$wt_path"
    echo "[ok] removed $wt_path (branch ${wt_branch} kept)"
done

git worktree prune
exit "$status"
