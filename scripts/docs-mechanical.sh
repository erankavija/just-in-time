#!/usr/bin/env bash
# NB: deliberately NOT `set -e`. All three checkers run even if an earlier one
# fails, so a single gate run reports every class of finding at once.
set -uo pipefail

# docs-mechanical — orchestrator for the `docs-mechanical` gate (epic 2d109173).
#
# Runs the three committed documentation mechanical checkers and aggregates
# their exit status (fail if any fails). Mirrors the cargo-ci.sh convention of
# one gate script fanning out to several independent steps.
#
#   M2  scripts/docs-check-links.sh        link + heading-anchor resolver
#   M3  scripts/docs-check-citations.sh    source-path + @/… citation existence
#   M5  scripts/docs-check-projections.sh  projection-freshness diff
#
# Any arguments are forwarded as the footprint to the link and citation checks
# (the projection check takes no footprint — its targets are configured). With
# no arguments each check uses its default full adopter-doc surface, which is
# what the gate runs.
#
# Exit codes:
#   0 — all three checks passed
#   1 — one or more checks reported findings
#   2 — a check hit an environment error

here=$(cd "$(dirname "$0")" && pwd)
status=0

run() {
  local label="$1"
  shift
  echo "== $label =="
  if ! "$@"; then
    status=1
  fi
  echo
}

run "M2 links & anchors" "$here/docs-check-links.sh" "$@"
run "M3 citations" "$here/docs-check-citations.sh" "$@"
run "M5 projections" "$here/docs-check-projections.sh"

exit "$status"
