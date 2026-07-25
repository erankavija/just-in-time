#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tool="$root/scripts/breakdown_manifest.py"
fixtures="$root/evals/fixtures"
config="$fixtures/hierarchy.toml"
conformance="$root/../../../crates/jit/tests/fixtures/batch-structural-conformance.json"
tmp_plan="$(mktemp)"
tmp_result="$(mktemp)"
trap 'rm -f "$tmp_plan" "$tmp_result"' EXIT

assert_rejects() {
  local expected="$1"
  shift
  local output status
  set +e
  output="$("$@" 2>&1)"
  status=$?
  set -e
  if ((status == 0)) || [[ "$output" != *"$expected"* ]]; then
    echo "expected rejection containing: $expected" >&2
    echo "$output" >&2
    return 1
  fi
}

"$tool" validate "$fixtures/manifest-valid.json" --config "$config" --known-source REQ-01 --required-source REQ-01 --required-criterion REQ-01 --plan "$fixtures/plan-stale.md" --deny-warnings
"$tool" validate "$fixtures/manifest-landing-group.json" --config "$config" --known-source REQ-02 --required-source REQ-02 --plan "$fixtures/plan-stale.md" --deny-warnings --json > "$tmp_result"
python3 -c 'import json,sys; result=json.load(open(sys.argv[1])); assert result["valid"] and result["advisories"]' "$tmp_result"
! "$tool" validate "$fixtures/manifest-valid.json" --config "$config" --known-source REQ-01 --required-source REQ-99 --plan "$fixtures/plan-stale.md"
! "$tool" validate "$fixtures/manifest-malformed.json" --config "$config" --known-source REQ-01
! "$tool" validate "$fixtures/manifest-malformed-types.json" --config "$config" --known-source REQ-01 --json > "$tmp_result"
python3 -c 'import json,sys; result=json.load(open(sys.argv[1])); assert result["errors"]' "$tmp_result"
! "$tool" validate "$fixtures/manifest-cyclic.json" --config "$config" --known-source REQ-01
assert_rejects "already implies" "$tool" validate "$fixtures/manifest-redundant-edge.json" --config "$config" --known-source REQ-01
# A cyclic graph reports the cycle alone; reachability would otherwise imply nearly every edge.
assert_rejects "contains a cycle" "$tool" validate "$fixtures/manifest-cyclic.json" --config "$config" --known-source REQ-01
! "$tool" validate "$fixtures/manifest-oversized.json" --config "$config" --known-source REQ-03 --deny-warnings
! "$tool" validate "$fixtures/manifest-independent-verbs.json" --config "$config" --known-source REQ-04 --deny-warnings
"$tool" validate "$fixtures/manifest-indivisible-evidence.json" --config "$config" --known-source REQ-05 --deny-warnings --json > "$tmp_result"
python3 -c 'import json,sys; result=json.load(open(sys.argv[1])); assert result["valid"] and result["overridden_warnings"]' "$tmp_result"

# Adversarial bypass class: no mandatory dimension may silently go unchecked.
assert_rejects "broad-quantifier" "$tool" validate "$fixtures/manifest-incant-indivisible.json" --config "$config" --known-source REQ-06 --deny-warnings
assert_rejects "cannot resolve terminal types" "$tool" validate "$fixtures/manifest-valid.json" --config "$fixtures/dropped-config.toml" --known-source REQ-01 --plan "$fixtures/plan-stale.md"
assert_rejects "unknown source refs: INVENTED-99" "$tool" validate "$fixtures/manifest-invent-source.json" --config "$config" --known-source REQ-08
assert_rejects "strictly finer in-manifest descendant" "$tool" validate "$fixtures/manifest-relabel-tier.json" --config "$config" --known-source REQ-07
cp "$fixtures/plan-stale.md" "$tmp_plan"
"$tool" validate "$fixtures/manifest-valid.json" --config "$config" --known-source REQ-01 --plan "$tmp_plan"
assert_rejects "contract ids must be unique" "$tool" validate "$fixtures/manifest-valid.json" --config "$config" --known-source REQ-01 --plan "$fixtures/plan-mislabel-contract-mode.md"
assert_rejects "malformed shared contract heading" "$tool" validate "$fixtures/manifest-valid.json" --config "$config" --known-source REQ-01 --plan "$fixtures/plan-mislabel-contract-mode.md"
assert_rejects "contract-like heading appears outside" "$tool" validate "$fixtures/manifest-valid.json" --config "$config" --known-source REQ-01 --plan "$fixtures/plan-mislabel-contract-mode.md"
"$tool" validate "$fixtures/manifest-contract-produced-valid.json" --config "$config" --known-source REQ-09 --plan "$fixtures/plan-contract-modes.md" --deny-warnings
assert_rejects "without depending on producer" "$tool" validate "$fixtures/manifest-contract-produced-unreachable.json" --config "$config" --known-source REQ-10 --plan "$fixtures/plan-contract-modes.md"
assert_rejects "must have exactly one producer" "$tool" validate "$fixtures/manifest-contract-invalid-producers.json" --config "$config" --known-source REQ-11 --plan "$fixtures/plan-contract-modes.md"
assert_rejects "cannot produce plan-fixed contract" "$tool" validate "$fixtures/manifest-contract-invalid-producers.json" --config "$config" --known-source REQ-11 --plan "$fixtures/plan-contract-modes.md"
assert_rejects "non-empty footprint" "$tool" validate "$fixtures/manifest-missing-footprint.json" --config "$config" --known-source REQ-12
! "$tool" render "$fixtures/manifest-valid.json" "$tmp_plan" --check
"$tool" render "$fixtures/manifest-valid.json" "$tmp_plan" --write
"$tool" render "$fixtures/manifest-valid.json" "$tmp_plan" --check
"$tool" conformance "$conformance"
