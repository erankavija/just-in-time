#!/usr/bin/env bash
set -euo pipefail

# docs-check-projections — projection-freshness diff.
# Mechanical check M5 of the `docs-mechanical` gate (epic 2d109173).
#
# Re-runs the two registry projectors IN PLACE and checks that their
# configured target regions still match the committed copy. A drifted region
# means the committed markdown is a stale hand-copy of a registry — the
# staleness defect `@/inv/single-source-prose` guards against.
#
# Both sides are derived live: the target paths come from the renderers' own
# `--json` output (`.target`), never a hardcoded path, and the "expected" side
# is produced by re-rendering the live registries. This check needs git (it
# diffs against the committed tree) — projection freshness is a git-tracked
# concern by definition.
#
# NOTE ON SIDE EFFECT: rendering writes to the target files. On a fresh tree
# the write is byte-identical (idempotent) so nothing changes; on a stale tree
# the working copy is left holding the freshly-rendered region, which is the
# fix ready to commit.
#
# Usage:
#   docs-check-projections.sh        (takes no footprint — targets are configured)
#
# Exit codes:
#   0 — projected regions are fresh  (prints "OK: projections fresh")
#   1 — a projected region drifted
#   2 — environment error (no git, render failed, jq missing)

command -v jq >/dev/null 2>&1 || {
  echo "docs-check-projections: 'jq' not found on PATH" >&2
  exit 2
}
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || {
  echo "docs-check-projections: not inside a git work tree (needed to diff committed copy)" >&2
  exit 2
}

if ! inv_json=$(jit invariant render --json); then
  echo "docs-check-projections: 'jit invariant render' failed" >&2
  exit 2
fi
if ! ref_json=$(jit reference render --json); then
  echo "docs-check-projections: 'jit reference render' failed" >&2
  exit 2
fi

inv_target=$(printf '%s' "$inv_json" | jq -r '.target')
ref_target=$(printf '%s' "$ref_json" | jq -r '.target')

if git diff --quiet -- "$inv_target" "$ref_target"; then
  echo "OK: projections fresh"
  exit 0
fi

echo "DRIFT: projected region(s) are stale — the committed copy does not match" >&2
echo "       'jit invariant render' / 'jit reference render'. Re-render and commit:" >&2
git --no-pager diff --stat -- "$inv_target" "$ref_target" >&2
exit 1
