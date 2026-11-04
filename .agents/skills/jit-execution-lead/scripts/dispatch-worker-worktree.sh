#!/usr/bin/env bash
# jit-execution-lead utility: create per-issue worktrees anchored to current
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
#   .agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh <short-id> [<short-id>...]
#
# Effects:
#   - Snapshots `git status -uall --porcelain` of main to
#     <dir>/lead-pre-dispatch-<ts>.txt and symlinks
#     <dir>/lead-pre-dispatch-latest.txt to it, where <dir> is
#     LEAD_SNAPSHOT_DIR, default /tmp.
#   - Creates one worktree per short-id at
#     .agents/worktrees/agent-<short-id> on branch
#     worktree-agent-<short-id> anchored to current main HEAD.
#   - Creates its temporary directory at
#     <LEAD_WORKTREE_TMP_ROOT>/<repo-path>/agent-<short-id>, defaulting
#     LEAD_WORKTREE_TMP_ROOT to /tmp/lead-worker-tmp. Override it with an
#     absolute path outside the repository; use the same value on reclaim.
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

# Build tools rewrite these in place or lock them (dep-info, build-script output,
# dot-named fingerprints, lock files and caches). A hardlink would carry those
# writes and locks into the pool and every sibling worktree, so each gets its own
# copy. Replacing the outermost match covers everything nested in it.
unshare_mutable_build_state() {
    local source="${1%/}" seeded="$2" entry
    find "$seeded" \( -name '.*' -o -type d -name build \) -prune -print0 \
        -o -type f -name '*.d' -print0 \
        | while IFS= read -r -d '' entry; do
            rm -rf "$entry"
            cp -a "$source${entry#"$seeded"}" "$entry"
        done
}

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

worker_tmp_root="${LEAD_WORKTREE_TMP_ROOT:-/tmp/lead-worker-tmp}"
if [[ "$worker_tmp_root" != /* ]]; then
    echo "ERROR: LEAD_WORKTREE_TMP_ROOT must be an absolute path." >&2
    exit 1
fi
worker_tmp_root="${worker_tmp_root%/}"

ts="$(date -u +%Y%m%d-%H%M%S)"
snapshot_dir="${LEAD_SNAPSHOT_DIR:-/tmp}"
snapshot="${snapshot_dir}/lead-pre-dispatch-${ts}.txt"
git status -uall --porcelain > "$snapshot"
ln -sfn "$snapshot" "${snapshot_dir}/lead-pre-dispatch-latest.txt"
snapshot_lines="$(wc -l < "$snapshot")"
echo "[ok] snapshot: $snapshot (${snapshot_lines} entries)"
echo "     latest -> ${snapshot_dir}/lead-pre-dispatch-latest.txt"

mkdir -p .agents/worktrees
created=()
for sid in "$@"; do
    wt_path=".agents/worktrees/agent-${sid}"
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

    worker_tmp_dir="$worker_tmp_root/${repo_root#/}/agent-${sid}"
    case "$worker_tmp_dir/" in
        "$repo_root/"*)
            echo "ERROR: worker temporary directory must be outside the repository: $worker_tmp_dir" >&2
            exit 1
            ;;
    esac
    mkdir -p "$worker_tmp_dir"

    pool="${LEAD_CACHE_POOL:-$repo_root/.agents/cache-pool}"
    if [[ -d "$pool" ]]; then
        for cache_dir in "$pool"/*/; do
            [[ -d "$cache_dir" ]] || continue
            name="$(basename "$cache_dir")"
            if cp -al "$cache_dir" "$wt_path/$name" 2>/dev/null; then
                unshare_mutable_build_state "$cache_dir" "$wt_path/$name"
                seed_mode="hardlink"
            else
                rm -rf "${wt_path:?}/${name:?}"
                cp -a "$cache_dir" "$wt_path/$name"
                seed_mode="copy"
            fi
            echo "[ok] $wt_path/$name seeded from pool ($seed_mode)"
        done
    fi

    echo "[ok] $wt_path  branch=$wt_branch  base=${short_main_sha}  tmp=$worker_tmp_dir"
    created+=("$sid")
done

cat <<EOF

==== Agent prompt header (paste at top of every dispatched prompt) ====
EOF

for sid in "${created[@]}"; do
    worker_tmp_dir="$worker_tmp_root/${repo_root#/}/agent-${sid}"
    cat <<EOF

---- prompt header for $sid ----
You are dispatched to JIT issue $sid. Your worktree is at:
  $repo_root/.agents/worktrees/agent-$sid
on branch worktree-agent-$sid, anchored to main at ${main_sha}.

Hard rules for path discipline (worktree-dispatch-protocol):
- Run every shell command from your worktree root. Prefix tool calls
  with 'cd $repo_root/.agents/worktrees/agent-$sid && ...' if your
  shell state has drifted.
- Use only paths relative to the worktree root in your tool calls.
  Checkout-specific absolute paths are forbidden — they leak
  files into main's checkout.
- Before every build and test command, export TMPDIR="$worker_tmp_dir".
- Never run 'git checkout', 'git switch', 'git worktree add/remove'.
- Commit on your worktree branch only. Do not push.
- Keep build/dependency caches at their in-worktree defaults; they may
  arrive pre-seeded warm. Never redirect one onto tmpfs.
EOF
done

echo
echo "Post-completion: run .agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh"
echo "to verify main's working tree did not absorb any worker output."
