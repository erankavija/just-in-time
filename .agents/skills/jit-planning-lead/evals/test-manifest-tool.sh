#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tool="$root/scripts/breakdown_manifest.py"
fixtures="$root/evals/fixtures"
tmp_plan="$(mktemp)"
tmp_result="$(mktemp)"
trap 'rm -f "$tmp_plan" "$tmp_result"' EXIT

"$tool" validate "$fixtures/manifest-valid.json" --terminal-type task --required-source REQ-01 --required-criterion REQ-01 --deny-warnings
"$tool" validate "$fixtures/manifest-landing-group.json" --terminal-type task --required-source REQ-02 --deny-warnings
! "$tool" validate "$fixtures/manifest-valid.json" --terminal-type task --required-source REQ-99
! "$tool" validate "$fixtures/manifest-malformed.json" --terminal-type task
! "$tool" validate "$fixtures/manifest-malformed-types.json" --terminal-type task --json > "$tmp_result"
python3 -c 'import json,sys; result=json.load(open(sys.argv[1])); assert result["errors"]' "$tmp_result"
! "$tool" validate "$fixtures/manifest-cyclic.json" --terminal-type task
! "$tool" validate "$fixtures/manifest-oversized.json" --terminal-type task --deny-warnings
! "$tool" validate "$fixtures/manifest-independent-verbs.json" --terminal-type task --deny-warnings
"$tool" validate "$fixtures/manifest-indivisible-evidence.json" --terminal-type task --deny-warnings
cp "$fixtures/plan-stale.md" "$tmp_plan"
"$tool" validate "$fixtures/manifest-valid.json" --terminal-type task --plan "$tmp_plan"
! "$tool" render "$fixtures/manifest-valid.json" "$tmp_plan" --check
"$tool" render "$fixtures/manifest-valid.json" "$tmp_plan" --write
"$tool" render "$fixtures/manifest-valid.json" "$tmp_plan" --check
