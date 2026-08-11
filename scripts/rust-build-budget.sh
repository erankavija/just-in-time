#!/usr/bin/env bash
# Rust build-footprint budget checker (jit:3f73423b).
#
# Derives the logical test topology and the active test-executable footprint
# from Cargo output — not from a directory scan of `target/`, which counts stale
# per-hash artifacts and cannot describe the current build — and fails when the
# repository exceeds its automatically enforced budgets or drifts from the
# intended build policies. `scripts/cargo-ci.sh` runs it after its normal Rust
# compilation so `cargo metadata` and `cargo test --no-run` reuse warm artifacts
# rather than triggering a second cold build (REQ-05).
#
# The inputs are injectable so every failure mode has an independent
# regression fixture that needs no compilation (REQ-04):
#   --root DIR           workspace root holding Cargo.toml, crates/jit/Cargo.toml
#                        and scripts/cargo-ci.sh (policy sources); default: the
#                        directory above this script.
#   --metadata-json FILE pre-captured `cargo metadata --no-deps
#                        --format-version=1`; default: run it live under --root.
#   --artifacts-json FILE pre-captured `cargo test --workspace --no-run
#                        --message-format=json`; default: run it live under
#                        --root. Executable paths are read from this stream and
#                        their on-disk sizes summed.
#   --test-suite-ms INTEGER measured nextest-plus-doctest suite duration;
#                        absent: skip this check.
#
# Exit codes:
#   0 — every budget and policy holds
#   1 — a budget or policy is violated (full diagnostics on stderr)
#   2 — environment problem (missing tool, unreadable input, empty artifact
#       stream): the check could not be performed, so it fails closed.
set -euo pipefail

# Budgets — declared once here and cited elsewhere, never re-copied into prose
# (@/inv/single-source-prose). Their justification and the measured evidence
# behind them live in the design doc's acceptance budgets, not in this file:
# dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md ("Artifact budget checker" and
# "Benchmark protocol").
readonly MAX_INTEGRATION_TARGETS=12
readonly MAX_EXECUTABLE_BYTES=$((2 * 1024 * 1024 * 1024)) # 2 GiB
readonly MAX_TEST_SUITE_SECONDS=30

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(dirname "$SCRIPT_DIR")
METADATA_JSON=""
ARTIFACTS_JSON=""
TEST_SUITE_MS=""
TEST_SUITE_MS_PROVIDED=false

usage() {
  sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --root) ROOT="$2"; shift 2 ;;
    --metadata-json) METADATA_JSON="$2"; shift 2 ;;
    --artifacts-json) ARTIFACTS_JSON="$2"; shift 2 ;;
    --test-suite-ms)
      if [ "$#" -lt 2 ]; then
        echo "rust-build-budget: missing value for --test-suite-ms" >&2
        exit 2
      fi
      TEST_SUITE_MS="$2"; TEST_SUITE_MS_PROVIDED=true; shift 2
      ;;
    -h | --help) usage; exit 0 ;;
    *)
      echo "rust-build-budget: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

command -v jq >/dev/null 2>&1 || {
  echo "rust-build-budget: 'jq' not found on PATH" >&2
  exit 2
}

# --- Input sourcing ---------------------------------------------------------
# Each input is either read from the injected fixture file or produced live by
# Cargo under $ROOT. Live invocations reuse the gate's warm target directory
# (REQ-05): `cargo metadata` performs no build, and after cargo-ci's test step
# `cargo test --no-run` only re-resolves and reports the executables already on
# disk.

read_metadata() {
  if [ -n "$METADATA_JSON" ]; then
    cat "$METADATA_JSON"
    return
  fi
  ( cd "$ROOT" && cargo metadata --no-deps --format-version=1 ) 2>/dev/null || {
    echo "rust-build-budget: 'cargo metadata' failed under $ROOT" >&2
    exit 2
  }
}

read_artifacts() {
  if [ -n "$ARTIFACTS_JSON" ]; then
    cat "$ARTIFACTS_JSON"
    return
  fi
  ( cd "$ROOT" && cargo test --workspace --no-run --message-format=json ) 2>/dev/null || {
    echo "rust-build-budget: 'cargo test --workspace --no-run' failed under $ROOT" >&2
    exit 2
  }
}

# Extract a scalar value for `key` under TOML `[section]` header from `file`.
# Returns the raw token (quotes/booleans preserved) or the empty string when
# absent. Handles the single, simply-nested policy keys this checker reads; not
# a general TOML parser.
toml_scalar() {
  local file="$1" section="$2" key="$3"
  [ -r "$file" ] || {
    echo "rust-build-budget: cannot read $file" >&2
    exit 2
  }
  awk -F' *= *' -v section="$section" -v key="$key" '
    /^[[:space:]]*\[/ {
      hdr = $0
      sub(/^[[:space:]]*\[/, "", hdr)
      sub(/\][[:space:]]*$/, "", hdr)
      insec = (hdr == section)
      next
    }
    insec && $1 == key {
      val = $2
      sub(/[[:space:]]*#.*$/, "", val)
      sub(/[[:space:]]*$/, "", val)
      print val
      exit
    }
  ' "$file"
}

# The single-line inline-table dependency declaration for `dep` in `file`
# (e.g. `ureq = { version = "3", default-features = false, ... }`), or empty.
dep_line() {
  local file="$1" dep="$2"
  [ -r "$file" ] || {
    echo "rust-build-budget: cannot read $file" >&2
    exit 2
  }
  grep -E "^[[:space:]]*${dep}[[:space:]]*=" "$file" | head -1
}

# --- Checks -----------------------------------------------------------------
errors=()

suite_duration_status="skipped"
if [ "$TEST_SUITE_MS_PROVIDED" = true ]; then
  if ! [[ "$TEST_SUITE_MS" =~ ^[0-9]+$ ]]; then
    echo "rust-build-budget: invalid --test-suite-ms value: expected a non-negative integer" >&2
    exit 2
  fi
  suite_duration_threshold_ms=$((MAX_TEST_SUITE_SECONDS * 1000))
  # Order by digit count before comparing arithmetically, because bash's (( ))
  # is 64-bit and wraps: `(( 9223372036854775808 >= 30000 ))` is false, so an
  # unguarded comparison lets a long enough value pass the budget it exceeds.
  # Stripping leading zeros makes digit count a magnitude order, so a wider
  # value exceeds the threshold without arithmetic and every value that reaches
  # (( )) fits the threshold's width.
  measured_ms="${TEST_SUITE_MS#"${TEST_SUITE_MS%%[!0]*}"}"
  measured_ms="${measured_ms:-0}"
  if [ "${#measured_ms}" -gt "${#suite_duration_threshold_ms}" ] \
    || { [ "${#measured_ms}" -eq "${#suite_duration_threshold_ms}" ] \
      && (( measured_ms >= suite_duration_threshold_ms )); }; then
    errors+=("test suite duration: observed ${TEST_SUITE_MS} ms, threshold ${suite_duration_threshold_ms} ms (must be below threshold).")
  else
    suite_duration_status="${TEST_SUITE_MS}ms"
  fi
fi

# REQ-01: integration-test target count from `cargo metadata`.
metadata=$(read_metadata)
target_count=$(printf '%s' "$metadata" \
  | jq '[.packages[].targets[] | select(.kind[] == "test")] | length')
if [ "$target_count" -gt "$MAX_INTEGRATION_TARGETS" ]; then
  errors+=("integration-test targets: observed ${target_count}, limit ${MAX_INTEGRATION_TARGETS}. Corrective area: consolidate top-level crates/jit/tests/*.rs entry points into cohesive suites (dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md, 'Test suite topology').")
fi

# REQ-02: unique active test-executable bytes from `cargo test --no-run`.
# Selection matches the recorded benchmark method (a non-null executable with
# profile.test == true); `sort -u` deduplicates paths before summing.
artifacts=$(read_artifacts)
mapfile -t executables < <(printf '%s' "$artifacts" \
  | jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null) | .executable' \
  | sort -u)
if [ "${#executables[@]}" -eq 0 ]; then
  echo "rust-build-budget: no active test executables in the artifact stream (expected at least one)" >&2
  exit 2
fi
executable_bytes=0
for exe in "${executables[@]}"; do
  size=$(stat -c %s "$exe" 2>/dev/null) || {
    echo "rust-build-budget: cannot stat active test executable: $exe" >&2
    exit 2
  }
  executable_bytes=$((executable_bytes + size))
done
if [ "$executable_bytes" -gt "$MAX_EXECUTABLE_BYTES" ]; then
  errors+=("active test executables: observed ${executable_bytes} bytes across ${#executables[@]} unique executables, limit ${MAX_EXECUTABLE_BYTES} bytes (2 GiB). Corrective area: shrink per-executable debug payload or consolidate suites (dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md, 'Artifact budget checker').")
fi

# REQ-03: build/gate policy assertions against the injected policy sources.
workspace_manifest="$ROOT/Cargo.toml"
jit_manifest="$ROOT/crates/jit/Cargo.toml"
gate_script="$ROOT/scripts/cargo-ci.sh"

# Debug profile: dev and test both bound to line tables (no full debugger
# payload). Mirrors the compile-time assertion in
# crates/jit/tests/scratch_build/build_profile_policy_tests.rs.
profile_mode="line-tables-only"
for profile in dev test; do
  debug=$(toml_scalar "$workspace_manifest" "profile.$profile" "debug")
  if [ "$debug" != "\"line-tables-only\"" ]; then
    errors+=("profile drift: [profile.${profile}].debug is ${debug:-unset}, expected \"line-tables-only\". Corrective area: restore the bounded debug info in ${workspace_manifest#"$ROOT"/} (@/inv/single-source-prose; see build_profile_policy_tests.rs).")
  fi
done

# Gate incremental policy: the gate script disables incremental compilation, so
# broad gate builds accumulate no incremental state.
incremental_mode="disabled"
if [ ! -r "$gate_script" ]; then
  echo "rust-build-budget: cannot read gate script $gate_script" >&2
  exit 2
fi
if ! grep -Eq '^[[:space:]]*export[[:space:]]+CARGO_INCREMENTAL=0' "$gate_script"; then
  errors+=("incremental-policy drift: ${gate_script#"$ROOT"/} no longer exports CARGO_INCREMENTAL=0. Corrective area: keep the gate's incremental compilation disabled (see build_profile_policy_tests.rs).")
fi

# Dependency-feature policy: no remote JSON Schema resolution, one TLS backend.
# Mirrors crates/jit/tests/scratch_build/dependency_feature_policy_tests.rs.
dep_feature_policy="pruned"
jsonschema_line=$(dep_line "$jit_manifest" "jsonschema")
if [ -z "$jsonschema_line" ]; then
  errors+=("dependency policy: no jsonschema dependency line found in ${jit_manifest#"$ROOT"/}. Corrective area: the resolver-feature check cannot run.")
else
  if ! printf '%s' "$jsonschema_line" | grep -Eq 'default-features[[:space:]]*=[[:space:]]*false'; then
    errors+=("remote resolver reintroduction: jsonschema does not set default-features = false, so its resolve-http/resolve-file/tls defaults return. Corrective area: disable default features on jsonschema (see dependency_feature_policy_tests.rs).")
  fi
  if printf '%s' "$jsonschema_line" | grep -Eq '(resolve-|tls-)'; then
    errors+=("remote resolver reintroduction: jsonschema explicitly enables a resolve-*/tls-* feature. Corrective area: drop remote-resolution features from jsonschema (see dependency_feature_policy_tests.rs).")
  fi
fi
ureq_line=$(dep_line "$jit_manifest" "ureq")
if [ -z "$ureq_line" ]; then
  errors+=("dependency policy: no ureq dependency line found in ${jit_manifest#"$ROOT"/}. Corrective area: the TLS-backend check cannot run.")
else
  if printf '%s' "$ureq_line" | grep -Eq '\bnative-tls\b'; then
    errors+=("duplicate TLS backend: ureq enables native-tls alongside rustls. Corrective area: select exactly one TLS backend for ureq (see dependency_feature_policy_tests.rs).")
  fi
  if ! printf '%s' "$ureq_line" | grep -Eq '\brustls\b'; then
    errors+=("TLS backend drift: ureq no longer enables rustls, the selected backend. Corrective area: keep exactly the rustls TLS backend for ureq (see dependency_feature_policy_tests.rs).")
  fi
fi

# --- Report -----------------------------------------------------------------
if [ "${#errors[@]}" -ne 0 ]; then
  echo "rust-build-budget: FAILED" >&2
  for e in "${errors[@]}"; do
    echo "  - $e" >&2
  done
  exit 1
fi

# REQ-06: one concise, parseable success line. cargo-ci.sh's summarize_pass
# folds it into the persisted gate summary.
echo "rust-build-budget: integration-targets=${target_count}/${MAX_INTEGRATION_TARGETS} active-executables=${#executables[@]} bytes=${executable_bytes}/${MAX_EXECUTABLE_BYTES} profile=${profile_mode} incremental=${incremental_mode} dep-features=${dep_feature_policy} suite-duration=${suite_duration_status}"
