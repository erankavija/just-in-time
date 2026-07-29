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
#   3. the default footprint: `docs`, the adopter documentation surface.
#
# The default is NOT read from `[documentation]` in `.jit/config.toml`. That
# table governs archival classification of the development root (`dev/`) —
# `permanent_paths` are the contributor areas an archive copies rather than
# relocates — and `docs/` is deliberately absent from it: a linked artifact
# outside the development root needs no archival destination and is retained
# where it is (`@/issue/8e071e18/decision/D-14`). So `[documentation]` has no
# field that names the adopter documentation root; there is nothing live to
# derive this default from, and `docs` is stated here as the one place this
# fact is declared for this checker. Contributor documentation under `dev/`
# is exactly what `[documentation].permanent_paths` denotes; a caller who
# wants that footprint passes it explicitly via positional args or
# DOCS_FOOTPRINT, same as any other non-default footprint (REQ-01, REQ-02).
#
# Exit codes (child semantics are preserved, exit 2 dominates — F4):
#   0 — all three checks passed
#   1 — one or more checks reported genuine findings (child exit 1) and no
#       check hit an environment error
#   2 — a check hit an environment/usage error (child exit 2 or other unexpected
#       nonzero); dominates a concurrent finding so a broken dependency /
#       renderer is never misreported as a documentation finding

here=$(cd "$(dirname "$0")" && pwd)

if [ "$#" -gt 0 ]; then
  FOOTPRINT=("$@")
elif [ -n "${DOCS_FOOTPRINT:-}" ]; then
  # Intentional word-split: DOCS_FOOTPRINT is a space-separated path list.
  # shellcheck disable=SC2206
  FOOTPRINT=($DOCS_FOOTPRINT)
else
  # The adopter documentation surface — see the footprint-resolution note above.
  FOOTPRINT=("docs")
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
