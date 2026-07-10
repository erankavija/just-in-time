#!/usr/bin/env bash
# NB: deliberately NOT `set -e`. All three checkers run even if an earlier one
# fails, so a single gate run reports every class of finding at once.
set -uo pipefail

# docs-mechanical — orchestrator for the `docs-mechanical` gate (epic 2d109173).
#
# Runs the three committed documentation mechanical checkers and aggregates
# their exit status. Mirrors the cargo-ci.sh convention of one gate script
# fanning out to several independent steps.
#
#   M2  scripts/docs-check-links.sh        link + heading-anchor resolver
#   M3  scripts/docs-check-citations.sh    source-path + @/… citation existence
#   M5  scripts/docs-check-projections.sh  projection-freshness diff
#
# Footprint resolution (passed to the two footprint-taking checkers; the
# projection check takes no footprint — its targets are configured):
#   1. positional args "$@", if any;                      else
#   2. the DOCS_FOOTPRINT env var (space-separated), if set — the lead's
#      per-issue scoping knob;                             else
#   3. a whole-surface footprint DERIVED LIVE from configuration (never a
#      hardcoded path list): the `[documentation].permanent_paths` roots, PLUS
#      repository-root and immediate-subdirectory markdown that lies outside the
#      contributor `development_root`. Both come from `jit config get
#      documentation`; the second set is expressed as a git-derived PATTERN
#      (tracked `*.md` at path depth ≤ 2, excluding the dev root and the
#      permanent roots), NOT a filename list — it re-derives itself when doc
#      roots move, so it encodes a pattern class, not product facts (REQ-01).
#
# Exit codes (child semantics are preserved, exit 2 dominates — F4):
#   0 — all three checks passed
#   1 — one or more checks reported genuine findings (child exit 1) and no
#       check hit an environment error
#   2 — a check hit an environment/usage error (child exit 2 or other unexpected
#       nonzero); dominates a concurrent finding so a broken dependency /
#       renderer is never misreported as a documentation finding

here=$(cd "$(dirname "$0")" && pwd)

# Resolve the whole-surface footprint live from configuration. Sets FOOTPRINT.
resolve_footprint() {
  command -v jit >/dev/null 2>&1 || {
    echo "docs-mechanical: 'jit' not found on PATH (needed to derive the footprint)" >&2
    exit 2
  }
  command -v jq >/dev/null 2>&1 || {
    echo "docs-mechanical: 'jq' not found on PATH (needed to derive the footprint)" >&2
    exit 2
  }
  local doc_json dev_root perm extra
  doc_json=$(jit config get documentation) || {
    echo "docs-mechanical: 'jit config get documentation' failed" >&2
    exit 2
  }
  dev_root=$(printf '%s' "$doc_json" | jq -r '.development_root')
  # (1) configured permanent documentation roots (currently docs/).
  perm=$(printf '%s' "$doc_json" | jq -r '.permanent_paths[]')
  # (2) root + immediate-subdir markdown OUTSIDE the contributor dev root and not
  # already covered by a permanent root. This is a PATTERN — tracked *.md at path
  # depth ≤ 2 minus the excluded roots — NOT a product fact / filename list.
  extra=$(git ls-files '*.md' | awk -F/ -v dev="$dev_root" -v perm="$perm" '
    BEGIN { n = split(perm, P, "\n") }
    NF > 2 { next }        # deeper than an immediate subdirectory
    $1 == dev { next }     # under the contributor development root
    {
      for (i = 1; i <= n; i++) {
        pr = P[i]; sub(/\/+$/, "", pr)               # strip trailing slash
        if (pr != "" && index($0, pr "/") == 1) next # covered by a permanent root
      }
      print
    }')
  FOOTPRINT=()
  while IFS= read -r p; do [ -n "$p" ] && FOOTPRINT+=("$p"); done <<<"$perm"
  while IFS= read -r f; do [ -n "$f" ] && FOOTPRINT+=("$f"); done <<<"$extra"
}

if [ "$#" -gt 0 ]; then
  FOOTPRINT=("$@")
elif [ -n "${DOCS_FOOTPRINT:-}" ]; then
  # Intentional word-split: DOCS_FOOTPRINT is a space-separated path list.
  # shellcheck disable=SC2206
  FOOTPRINT=($DOCS_FOOTPRINT)
else
  resolve_footprint
fi

env_error=0
finding=0

run() {
  local label="$1"
  shift
  echo "== $label =="
  "$@"
  local rc=$?
  case "$rc" in
    0) ;;
    1) finding=1 ;;
    *) env_error=1 ;; # exit 2 (env/usage) or any unexpected nonzero
  esac
  echo
}

run "M2 links & anchors" "$here/docs-check-links.sh" "${FOOTPRINT[@]}"
run "M3 citations" "$here/docs-check-citations.sh" "${FOOTPRINT[@]}"
run "M5 projections" "$here/docs-check-projections.sh"

if [ "$env_error" -eq 1 ]; then
  exit 2
elif [ "$finding" -eq 1 ]; then
  exit 1
fi
exit 0
