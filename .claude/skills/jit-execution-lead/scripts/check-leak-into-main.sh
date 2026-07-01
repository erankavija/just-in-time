#!/usr/bin/env bash
# Project-lead utility: after parallel worktree-dispatch agents
# complete, verify main's working tree contains nothing that wasn't
# there at dispatch time. Files written by workers should live in
# their worktrees, not in main.
#
# Pairs with dispatch-worker-worktree.sh in this same directory.
# Operates on whatever git repo the current working directory
# belongs to.
#
# Reference: references/worktree-dispatch-protocol.md (in this skill).
#
# Usage:
#   scripts/check-leak-into-main.sh [<snapshot-path>]
#     snapshot-path defaults to /tmp/lead-pre-dispatch-latest.txt
#
# Exit codes:
#   0 — main matches snapshot (no worker leaks)
#   1 — main differs from snapshot (leaks present; details listed)
#   2 — bad invocation (snapshot missing)

set -euo pipefail

snapshot="${1:-/tmp/lead-pre-dispatch-latest.txt}"

if [[ ! -e "$snapshot" ]]; then
    echo "ERROR: snapshot $snapshot not found. Run scripts/dispatch-worker-worktree.sh first." >&2
    exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

current="$(mktemp)"
trap 'rm -f "$current"' EXIT
git status -uall --porcelain > "$current"

if cmp -s "$snapshot" "$current"; then
    echo "[ok] main is clean — no worker leaks detected (snapshot: $snapshot)"
    exit 0
fi

echo "[!] main's working tree differs from pre-dispatch snapshot."
echo "    snapshot: $snapshot"
echo

added="$(comm -13 <(sort "$snapshot") <(sort "$current") || true)"
removed="$(comm -23 <(sort "$snapshot") <(sort "$current") || true)"

if [[ -n "$added" ]]; then
    echo "Entries present now that were NOT in the snapshot (likely worker leaks):"
    printf '%s\n' "$added" | sed 's/^/    /'
    echo
fi

if [[ -n "$removed" ]]; then
    echo "Entries that disappeared between snapshot and now:"
    printf '%s\n' "$removed" | sed 's/^/    /'
    echo
fi

cat <<'EOF'
Recovery options:
  Tracked file modification (' M <path>'):
      git restore <path>
  Untracked file ('?? <path>'):
      mv <path> <correct-worktree-path>
      # (then amend the worker's preserve commit to include it)
    or
      rm <path>
      # if truly unwanted

After recovery, re-run this script to confirm main is clean.
EOF
exit 1
