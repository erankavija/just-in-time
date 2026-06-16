#!/usr/bin/env bash
set -euo pipefail

# Coverage-preview gate checker for jit (planning bracket, T6 / D8 / D13).
#
# Runs scoped validation for the container `C` bracketed by the breakdown node
# `B` this gate is attached to. `C` is recovered from `B`'s `brackets:<C-id>`
# label, then the deterministic scope validator is invoked:
#
#   jit validate --scope <C>
#
# which exits 4 (and prints findings) when the drafted children leave a
# `[hard]` criterion uncovered at plan time, and 0 when coverage is complete.
#
# Contract:
#   exit 0  — gate passed (scope validation clean)
#   exit 1  — checker error (could not resolve the container)
#   other   — propagated from `jit validate --scope` (4 = validation failed)
#   stdout  — validation output, captured by jit in gate run results
#   stderr  — errors, shown by jit on failure
#
# Requires:
#   JIT_ISSUE_ID — set automatically by jit to the gated issue (the breakdown
#                  node B). The container is resolved from B's brackets: label.
#   jq           — used to extract the brackets: label from `jit issue show`.

if [[ -z "${JIT_ISSUE_ID:-}" ]]; then
  echo "coverage-preview: JIT_ISSUE_ID is not set (run via a jit gate)" >&2
  exit 1
fi

# Read the breakdown node's labels and pull the brackets:<C-id> pointer.
container=$(jit issue show "$JIT_ISSUE_ID" --json \
  | jq -r '.labels[] | select(startswith("brackets:")) | sub("^brackets:"; "")' \
  | head -n1)

if [[ -z "$container" ]]; then
  echo "coverage-preview: issue $JIT_ISSUE_ID has no brackets:<C-id> label;" \
       "it is not a breakdown node bracketing a container" >&2
  exit 1
fi

# Deterministic scope validation for the resolved container. Exit code (incl.
# 4 = ValidationFailed) propagates to jit, which decides pass/fail.
exec jit validate --scope "$container"
