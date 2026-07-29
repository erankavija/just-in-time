#!/usr/bin/env bash
set -euo pipefail

# Cargo CI gate wrapper for jit.
#
# Runs the Rust CI pipeline (fmt check, zero-warning clippy, workspace tests)
# and produces CONCISE output: one-line summaries on success, full diagnostics
# only on failure.
#
# Why a wrapper instead of the raw `cargo fmt && clippy && test` command:
# the `code-review` gate ingests this gate's stored stdout as the authoritative
# test-run evidence (`--pass-context` run history). The raw `cargo test
# --workspace` output is thousands of lines (every test name across every
# binary); dumping that verbatim floods the reviewer's context and pushes it to
# distrust the evidence and re-run tests itself in a restricted sandbox where
# environment-sensitive tests (port binding, process spawn, concurrency) fail.
# A clean "N passed, 0 failed" summary is trustworthy evidence and keeps
# .jit/gate-runs/ small. Modelled on ../gf2/scripts/cargo-ci.sh.
#
# Exit codes:
#   0 — all steps passed
#   1 — one or more steps failed
#   2 — environment problem: no real cargo available on PATH

# Host-wide build serialization. Only one cargo-ci run executes the heavy
# build/test steps at a time across the whole host. Concurrent gate runs (e.g.
# several agent sessions each calling `jit gate evaluate ... cargo-ci`) otherwise
# oversubscribe the CPU — every `cargo build` fans out to all cores, so K runs
# demand K×nproc — and multiply peak RAM into swap, making the host and any
# interactive shell laggy. Serializing also protects the load-sensitive
# concurrency proptest (prop_concurrent_different_issues_succeed) from starving
# its file-lock timeout under saturation. We re-exec the script under a blocking
# flock so concurrent runs queue rather than fail; the lock is held for the
# whole run and released when the process exits. CARGO_CI_LOCKED guards against
# infinite re-exec; CARGO_CI_NO_LOCK=1 disables (e.g. an isolated CI container
# that already owns the machine); CARGO_CI_BUILD_LOCK overrides the lock path.
if [ -z "${CARGO_CI_NO_LOCK:-}" ] && [ -z "${CARGO_CI_LOCKED:-}" ]; then
  BUILD_LOCK="${CARGO_CI_BUILD_LOCK:-${XDG_RUNTIME_DIR:-/tmp}/jit-cargo-ci.lock}"
  if command -v flock >/dev/null 2>&1; then
    exec env CARGO_CI_LOCKED=1 flock "$BUILD_LOCK" "$0" "$@"
  fi
  echo "cargo-ci: flock not found; running without host-wide build lock" >&2
fi

# Resolve the real cargo binary. Some local setups place a debugging shim at
# ~/.cargo/bin/cargo that exits 0 for every invocation; without this guard each
# cargo step below would silently succeed, recording a false-positive gate PASS.
# Detect a stub via the canonical `cargo X.Y.Z` version-probe, then fall back to
# a rustup toolchain binary. Fail loudly (exit 2) rather than rubber-stamp.
# (Pattern from jit-adjacent gf2; see gf2 issue 941d1528.)
ensure_real_cargo() {
  local probe
  probe=$(cargo --version 2>&1 || true)
  if [[ "$probe" =~ ^cargo[[:space:]][0-9]+\.[0-9]+\.[0-9]+ ]]; then
    return 0
  fi
  for tc_dir in "$HOME/.rustup/toolchains"/stable-*; do
    [[ -d "$tc_dir" && -x "$tc_dir/bin/cargo" ]] || continue
    export PATH="$tc_dir/bin:$PATH"
    probe=$(cargo --version 2>&1 || true)
    if [[ "$probe" =~ ^cargo[[:space:]][0-9]+\.[0-9]+\.[0-9]+ ]]; then
      echo "cargo-ci: cargo on PATH was a stub; using $tc_dir/bin/cargo" >&2
      return 0
    fi
  done
  echo "ERROR: no real cargo on PATH and no usable rustup stable toolchain found." >&2
  echo "       cargo --version output: $probe" >&2
  exit 2
}

ensure_real_cargo

# Disk-backed TMPDIR: a few tests (provenance_contract suite) compile the whole crate
# into a fresh temp target dir; on a small tmpfs /tmp that hits "Disk quota
# exceeded". Use a disk-backed cache dir. It must live OUTSIDE any git repo:
# tests such as test_get_current_branch_errors_when_git_fails create a temp dir
# here and expect no enclosing `.git` (so $PWD/target — inside this repo — is
# wrong). Overridable via CARGO_CI_TMPDIR.
export TMPDIR="${CARGO_CI_TMPDIR:-${XDG_CACHE_HOME:-$HOME/.cache}/jit-cargo-ci-tmp}"
mkdir -p "$TMPDIR"

# Disable incremental compilation for every step below (jit:57d0eb79). The
# workspace manifest's [profile.dev]/[profile.test] leave incremental on for
# ordinary interactive builds, where it earns back its disk cost across many
# rebuilds of the same tree. A gate run compiles once and exits, so it has no
# later rebuild to amortize that cost against; left on, incremental state
# accumulated without bound across gate runs (baseline measurement:
# dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md, Baseline table). The
# `incremental-state` step below turns "should be disabled" into a checked
# fact rather than an assumption.
export CARGO_INCREMENTAL=0

# Parallel test harness across 20 threads. The suite is I/O-bound (the
# storage::claim_coordinator proptests do real filesystem locking over hundreds
# of cases each), so serial execution pushed the gate past two minutes. Capped
# at 20 (not nproc) to leave headroom: a pre-existing load-sensitive concurrency
# proptest (prop_concurrent_different_issues_succeed) can starve its file-lock
# timeout when every core is saturated by parallel build+test load. Overridable.
export RUST_TEST_THREADS="${RUST_TEST_THREADS:-20}"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

failed=0
summary=""

summarize_pass() {
  local name="$1"
  case "$name" in
    test | provenance)
      # cargo test runs many binaries, each printing its own
      # "test result: ok. N passed; M failed; K ignored; ...". Sum them.
      local p f i
      p=$(grep -oP 'test result: ok\. \K\d+(?= passed)' "$WORK/$name.out" \
            | awk '{s+=$1} END {print s+0}')
      f=$(grep -oP '\K\d+(?= failed)' "$WORK/$name.out" \
            | awk '{s+=$1} END {print s+0}')
      i=$(grep -oP '\K\d+(?= ignored)' "$WORK/$name.out" \
            | awk '{s+=$1} END {print s+0}')
      # REQ-04 (jit:83efbcb4): the provenance step's own $WORK/provenance.out
      # is deleted by the EXIT trap once this script finishes, so surface its
      # seeded-fixture file/byte count line (printed via --nocapture) into the
      # persisted gate summary here rather than leaving it observable only in
      # a temp file that is already gone by the time anyone reads this output.
      local fixture=""
      if [ "$name" = "provenance" ]; then
        fixture=$(grep -o 'provenance-fixture: files=[0-9]* bytes=[0-9]*' "$WORK/$name.out" | tail -1)
        [ -n "$fixture" ] && fixture=" ($fixture)"
      fi
      echo "${p:-0} passed, ${f:-0} failed, ${i:-0} ignored${fixture}"
      ;;
    budget)
      # REQ-06 (jit:3f73423b): fold the checker's one-line build-footprint
      # summary into the persisted gate summary (its $WORK/budget.out is deleted
      # by the EXIT trap). The "rust-build-budget: " prefix is stripped since the
      # "✓ budget:" label already identifies the step.
      grep -o 'rust-build-budget: integration-targets=.*' "$WORK/$name.out" \
        | tail -1 | sed 's/^rust-build-budget: //'
      ;;
    *)
      echo "ok"
      ;;
  esac
}

summarize_fail() {
  local name="$1"
  case "$name" in
    test | provenance)
      echo "--- $name failures ---"
      # Failed test names and the captured panic/assert output blocks.
      grep -E '^test .* FAILED$' "$WORK/$name.out" || true
      grep -E '^test result: FAILED' "$WORK/$name.out" || true
      awk '/^---- .* ----$/{p=1} p{print} /^$/{if(p)c++; if(c>1)p=0}' \
        "$WORK/$name.out" | head -80 || true
      ;;
    clippy)
      echo "--- $name diagnostics ---"
      grep -E '^(warning|error)' "$WORK/$name.out" | head -60 || true
      ;;
    fmt)
      echo "--- $name diff ---"
      head -80 "$WORK/$name.out"
      ;;
    *)
      echo "--- $name output ---"
      tail -40 "$WORK/$name.out"
      ;;
  esac
}

run_step() {
  local name="$1"
  shift
  if "$@" >"$WORK/$name.out" 2>&1; then
    summary+="  ✓ $name: $(summarize_pass "$name")"$'\n'
  else
    local rc=$?
    summary+="  ✗ $name: FAILED (exit $rc)"$'\n'
    summarize_fail "$name"
    failed=1
  fi
}

# REQ-04 (jit:57d0eb79): the target directory this run actually used —
# CARGO_TARGET_DIR when the caller set one (e.g. the isolated-run
# verification protocol), else Cargo's own resolved default. Reading it from
# `cargo metadata` rather than assuming "$workspace_root/target" respects any
# .cargo/config.toml override.
gate_target_dir() {
  if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    printf '%s\n' "$CARGO_TARGET_DIR"
    return
  fi
  command -v jq >/dev/null 2>&1 || {
    echo "gate_target_dir: 'jq' not found on PATH (needed to resolve Cargo's default target directory)" >&2
    return 1
  }
  cargo metadata --format-version=1 --no-deps 2>/dev/null | jq -r '.target_directory'
}

# REQ-04 (jit:57d0eb79): CARGO_INCREMENTAL=0 above is the mechanism; this is
# the deterministic check that it held. Runs after the compilation steps so
# it observes what they actually left on disk. Suites that spawn scratch
# builds into their own throwaway target dirs (scratch_build,
# provenance_contract) are out of scope by construction: this only walks the
# directory `gate_target_dir` resolves, never a suite's private scratch dir.
check_no_incremental_state() {
  local target_dir
  target_dir=$(gate_target_dir) || return 1
  if [ -z "$target_dir" ]; then
    echo "could not resolve this run's Cargo target directory" >&2
    return 1
  fi
  local hits
  hits=$(find "$target_dir" -type d -name incremental -not -empty 2>/dev/null)
  if [ -n "$hits" ]; then
    echo "non-empty incremental directories under $target_dir:" >&2
    echo "$hits" >&2
    return 1
  fi
}

# Deprioritize the build/test work so an interactive shell preempts it under
# contention — this is what keeps the host responsive while the gate runs, not
# just the serialization above. nice -n 19 = lowest CPU priority; ionice -c2 -n7
# = best-effort lowest I/O priority (NOT the idle class -c3, which can be starved
# indefinitely and would risk the timing-sensitive file-lock proptests). Both
# are best-effort: a missing binary degrades gracefully to running normally.
# CARGO_CI_NO_NICE=1 disables; CARGO_CI_NICE overrides the niceness.
NICE_PREFIX=()
if [ -z "${CARGO_CI_NO_NICE:-}" ]; then
  command -v nice   >/dev/null 2>&1 && NICE_PREFIX+=(nice -n "${CARGO_CI_NICE:-19}")
  command -v ionice >/dev/null 2>&1 && NICE_PREFIX+=(ionice -c2 -n7)
fi

# Run all steps (continue through failures so every problem is reported, unlike
# a short-circuiting `&&` chain). Same checks the inline gate ran.
# Reject pre-existing incremental state before paying for any compilation. The
# final incremental-state step remains authoritative for state created during
# this run; this preflight only avoids an expensive gate that is already known
# to be unable to pass.
run_step incremental-preflight check_no_incremental_state
if [ "$failed" -ne 0 ]; then
  echo "$summary"
  exit 1
fi

run_step fmt    "${NICE_PREFIX[@]}" cargo fmt --all -- --check
run_step clippy "${NICE_PREFIX[@]}" cargo clippy --workspace --all-targets -- -D warnings
run_step test   "${NICE_PREFIX[@]}" cargo test --workspace

# Build-provenance contract suites (jit:5d862134). These are #[ignore]d for plain
# `cargo test` — each spawns cold scratch `cargo` builds into throwaway target
# dirs to exercise the build script under real git states, costing ~3-4 min that
# ordinary dev runs should not pay — so the `test` step above skips them. The gate
# paying that cost is exactly the point: REQ-06's hard metadata-only-invalidation
# and injected-provenance contracts are unexercised unless a required CI step runs
# them, so this step does. No lock interaction: these tests spawn plain `cargo`
# only (never scripts/cargo-ci.sh or verify-commit-builds.sh), so they do not
# re-acquire the CARGO_CI_BUILD_LOCK this run already holds.
# --nocapture (jit:83efbcb4 REQ-04): the metadata-stability test prints the
# seeded-fixture file/byte count line before its cold build; this flag is what
# lets that line reach $WORK/provenance.out for summarize_pass to fold into
# the persisted gate summary below, on a passing run and not just a failure.
run_step provenance "${NICE_PREFIX[@]}" cargo test -p jit \
  --test provenance_contract -- --ignored --nocapture

# Build-footprint budget enforcement (jit:3f73423b). Runs AFTER the test step so
# its `cargo metadata` and `cargo test --workspace --no-run --message-format=json`
# reuse this run's warm target directory rather than triggering a second cold
# build (REQ-05): metadata performs no build, and --no-run only re-resolves and
# reports the test executables the `test` step already linked. The checker
# derives the integration-target count and unique active-executable bytes from
# that Cargo output, and asserts the debug-profile, gate-incremental, and
# dependency-feature policies against the committed manifests and this script.
# It self-locates the workspace root from its own path, so no argument is needed
# for a live gate run. CARGO_INCREMENTAL=0 (exported above) is inherited, so its
# warm --no-run leaves no incremental state for the check below.
CARGO_CI_SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
run_step budget "${NICE_PREFIX[@]}" "$CARGO_CI_SCRIPT_DIR/rust-build-budget.sh"

# REQ-04 (jit:57d0eb79): fail the gate itself if the compilation steps above
# left behind incremental state, rather than trusting that CARGO_INCREMENTAL=0
# held.
run_step incremental-state check_no_incremental_state

echo "$summary"

if [ "$failed" -ne 0 ]; then
  exit 1
fi
