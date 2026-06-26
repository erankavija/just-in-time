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
# several agent sessions each calling `jit gate pass ... cargo-ci`) otherwise
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

# Disk-backed TMPDIR: a few tests (version_cli_tests) compile the whole crate
# into a fresh temp target dir; on a small tmpfs /tmp that hits "Disk quota
# exceeded". Use a disk-backed cache dir. It must live OUTSIDE any git repo:
# tests such as test_get_current_branch_errors_when_git_fails create a temp dir
# here and expect no enclosing `.git` (so $PWD/target — inside this repo — is
# wrong). Overridable via CARGO_CI_TMPDIR.
export TMPDIR="${CARGO_CI_TMPDIR:-${XDG_CACHE_HOME:-$HOME/.cache}/jit-cargo-ci-tmp}"
mkdir -p "$TMPDIR"

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
    test)
      # cargo test runs many binaries, each printing its own
      # "test result: ok. N passed; M failed; K ignored; ...". Sum them.
      local p f i
      p=$(grep -oP 'test result: ok\. \K\d+(?= passed)' "$WORK/$name.out" \
            | awk '{s+=$1} END {print s+0}')
      f=$(grep -oP '\K\d+(?= failed)' "$WORK/$name.out" \
            | awk '{s+=$1} END {print s+0}')
      i=$(grep -oP '\K\d+(?= ignored)' "$WORK/$name.out" \
            | awk '{s+=$1} END {print s+0}')
      echo "${p:-0} passed, ${f:-0} failed, ${i:-0} ignored"
      ;;
    *)
      echo "ok"
      ;;
  esac
}

summarize_fail() {
  local name="$1"
  case "$name" in
    test)
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
run_step fmt    "${NICE_PREFIX[@]}" cargo fmt --all -- --check
run_step clippy "${NICE_PREFIX[@]}" cargo clippy --workspace --all-targets -- -D warnings
run_step test   "${NICE_PREFIX[@]}" cargo test --workspace

echo "$summary"

if [ "$failed" -ne 0 ]; then
  exit 1
fi
