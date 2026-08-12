#!/usr/bin/env bash
set -euo pipefail

# Cargo CI gate wrapper for jit.
#
# Runs the Rust CI pipeline (fmt check, zero-warning clippy, pinned nextest
# workspace tests, and doctests) and produces CONCISE output: one-line summaries
# on success, full diagnostics only on failure.
#
# Why a wrapper instead of the raw `cargo fmt && clippy && test` command:
# the `code-review` gate ingests this gate's stored stdout as the authoritative
# test-run evidence (`--pass-context` run history). The raw `cargo nextest run
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
#   2 — environment problem: Cargo prerequisites are missing or mismatched
#
# `./scripts/cargo-ci.sh --cargo <args...>` applies the same host lock, real
# Cargo selection, and guarded sccache setup, then runs only that focused Cargo
# command. Use this mode for compilation-heavy targeted checks; invoke the
# script without arguments only when the complete gate is intended.

# Host-wide build serialization. Only one cargo-ci run executes the heavy
# build/test steps at a time across the whole host. Concurrent gate runs (e.g.
# several agent sessions each calling `jit gate evaluate ... cargo-ci`) otherwise
# oversubscribe the CPU — every `cargo build` fans out to all cores, so K runs
# demand K×nproc — and multiply peak RAM into swap, making the host and any
# interactive shell laggy. We re-run the script under a blocking
# flock so concurrent runs queue rather than fail; the lock is held for the
# whole run and released when the process exits. CARGO_CI_LOCKED guards against
# infinite re-exec; CARGO_CI_NO_LOCK=1 disables (e.g. an isolated CI container
# that already owns the machine); CARGO_CI_BUILD_LOCK overrides the lock path;
# CARGO_CI_LOCK_TIMEOUT bounds the wait.
#
# `flock -o` is load-bearing: it closes the lock descriptor in the child before
# exec. Without it every descendant inherits the descriptor, and a descendant
# that daemonises keeps holding the lock after this run exits — `sccache`
# double-forks to PPID 1 and does exactly that — so the next caller blocks
# forever against a run that finished long ago. The lock releases when the
# waiting `flock` parent exits, which is the intended "held for the whole run"
# lifetime.
#
# The wait is bounded and announced. An unbounded silent block is
# indistinguishable from a hung or dead process, which is how a wedged lock
# stayed undiagnosed for minutes at a time; `-E 75` separates "could not acquire
# the lock" from the wrapped command's own exit status so the timeout can name
# what it was waiting for.
report_build_lock_holders() {
  local lock="$1"
  if command -v lslocks >/dev/null 2>&1; then
    lslocks -o COMMAND,PID,MODE,PATH 2>/dev/null | awk -v lock="$lock" 'NR==1 || $NF==lock'
  elif command -v fuser >/dev/null 2>&1; then
    fuser -v "$lock" 2>&1
  fi
}

if [ -z "${CARGO_CI_NO_LOCK:-}" ] && [ -z "${CARGO_CI_LOCKED:-}" ]; then
  BUILD_LOCK="${CARGO_CI_BUILD_LOCK:-${XDG_RUNTIME_DIR:-/tmp}/cargo-ci.lock}"
  LOCK_TIMEOUT="${CARGO_CI_LOCK_TIMEOUT:-1800}"
  if command -v flock >/dev/null 2>&1; then
    if ! flock -n -o "$BUILD_LOCK" true 2>/dev/null; then
      echo "cargo-ci: another build holds $BUILD_LOCK; waiting up to ${LOCK_TIMEOUT}s" >&2
      report_build_lock_holders "$BUILD_LOCK" >&2
    fi
    lock_status=0
    env CARGO_CI_LOCKED=1 flock -o -w "$LOCK_TIMEOUT" -E 75 "$BUILD_LOCK" "$0" "$@" ||
      lock_status=$?
    if [ "$lock_status" -eq 75 ]; then
      echo "ERROR: cargo-ci: timed out after ${LOCK_TIMEOUT}s waiting for $BUILD_LOCK" >&2
      report_build_lock_holders "$BUILD_LOCK" >&2
      echo "       A holder with PPID 1 and no live cargo/rustc is a leaked daemon," >&2
      echo "       not a running build; stop it (e.g. sccache --stop-server) and retry." >&2
      exit 2
    fi
    exit "$lock_status"
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

# Reuse the host-wide compiler cache when it is installed. Keep explicit
# wrappers authoritative (for instrumentation or debugging), and provide a
# deterministic opt-out for cache-sensitive diagnosis. Enabling a wrapper
# changes Cargo fingerprints, so operators should clean a large pre-existing
# target directory before opting an established checkout into this default.
if [ -z "${CARGO_CI_NO_SCCACHE:-}" ] && [ -z "${RUSTC_WRAPPER:-}" ] && command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache
fi

if [ "${1:-}" = "--cargo" ]; then
  shift
  exec cargo "$@"
fi

# The concise reporter below parses cargo-nextest's human summary contract.
# Fail before creating gate scratch state or running any step unless the exact
# version provisioned by CI and committed in .config/nextest.toml is active.
readonly PINNED_NEXTEST_VERSION="0.9.133"
ensure_pinned_nextest() {
  local probe expected
  probe=$(cargo nextest --version 2>&1 || true)
  expected="cargo-nextest $PINNED_NEXTEST_VERSION"
  if [ "$probe" != "$expected" ] && [[ "$probe" != "$expected "* ]]; then
    echo "ERROR: cargo-ci requires cargo-nextest $PINNED_NEXTEST_VERSION; reporter parsing is pinned to that version." >&2
    echo "       cargo nextest --version output: $probe" >&2
    exit 2
  fi
}

ensure_pinned_nextest

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

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

failed=0
summary=""

# Gate evidence is stored as text, so keep all reported wall-clock values in
# one unambiguous, machine-friendly unit. GNU date's %3N expansion is always a
# three-digit millisecond field on the Linux hosts that run this gate.
epoch_milliseconds() {
  date +%s%3N
}

summarize_pass() {
  local name="$1"
  case "$name" in
    test)
      # Pinned nextest emits one final line shaped as:
      # "Summary [...] N tests run: P passed[, F failed][, S skipped]".
      # Treat a missing/unparseable line as a reporter-contract failure rather
      # than persisting a misleading zero-count pass.
      local reporter p f s
      reporter=$(grep -E 'Summary .* [0-9]+ tests run:' "$WORK/$name.out" | tail -1 || true)
      p=$(grep -oP '\K[0-9]+(?= passed)' <<<"$reporter" | tail -1 || true)
      f=$(grep -oP '\K[0-9]+(?= failed)' <<<"$reporter" | tail -1 || true)
      s=$(grep -oP '\K[0-9]+(?= skipped)' <<<"$reporter" | tail -1 || true)
      if [ -z "$reporter" ] || [ -z "$p" ]; then
        echo "cargo-ci: could not parse pinned nextest success summary" >&2
        return 1
      fi
      echo "$p passed, ${f:-0} failed, ${s:-0} skipped"
      ;;
    doctest)
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
    test)
      echo "--- $name failures ---"
      # Preserve nextest's semantic test identity/timing records and final
      # totals. Tail output retains the captured assertion/panic diagnostics;
      # the explicit greps keep identities/totals visible for a broad suite.
      grep -E '^[[:space:]]*(FAIL|ABORT|TIMEOUT) \[' "$WORK/$name.out" | head -60 || true
      grep -E '^[[:space:]]*Summary .* tests run:' "$WORK/$name.out" | tail -1 || true
      tail -80 "$WORK/$name.out"
      ;;
    doctest)
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
  local started_ms ended_ms elapsed_ms
  shift

  started_ms=$(epoch_milliseconds)
  if "$@" >"$WORK/$name.out" 2>&1; then
    ended_ms=$(epoch_milliseconds)
    elapsed_ms=$((ended_ms - started_ms))
    local passed_summary
    if passed_summary=$(summarize_pass "$name"); then
      summary+="  ✓ $name: $passed_summary (${elapsed_ms} ms)"$'\n'
    else
      summary+="  ✗ $name: REPORTER FAILED (${elapsed_ms} ms)"$'\n'
      summarize_fail "$name"
      failed=1
    fi
  else
    local rc=$?
    ended_ms=$(epoch_milliseconds)
    elapsed_ms=$((ended_ms - started_ms))
    summary+="  ✗ $name: FAILED (exit $rc) (${elapsed_ms} ms)"$'\n'
    summarize_fail "$name"
    failed=1
  fi
}

# The workspace this run actually compiled. Cargo resolves it by walking up from
# the current directory, so a run started from a subdirectory — or through
# another checkout's scripts/ — still names the tree the steps below judge. The
# budget checker is pointed at it rather than at this script's own checkout, so
# one run judges one workspace.
gate_workspace_root() {
  command -v jq >/dev/null 2>&1 || {
    echo "gate_workspace_root: 'jq' not found on PATH (needed to resolve the Cargo workspace root)" >&2
    return 1
  }
  cargo metadata --format-version=1 --no-deps 2>/dev/null | jq -r '.workspace_root'
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
# builds into their own throwaway target dirs (scratch_build) are out of scope
# by construction: this only walks the directory `gate_target_dir` resolves,
# never a suite's private scratch dir.
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
# = best-effort lowest I/O priority (NOT the idle class -c3, whose requests can be
# starved for as long as any other class has I/O pending, leaving the gate itself
# without a bound on when it finishes). Both are best-effort: a missing binary
# degrades gracefully to running normally.
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

# Compile the measured suite before the clock starts (jit:94d85bf1). The budget
# enforced below is defined over an already-built target, so compilation paid
# inside the clock would fail the gate on every cold one — every fresh clone and
# every CI runner — with nothing wrong in the tree. The recorded cargo-ci run
# for jit:6d10e5d4 at commit e2da055a reports a 239,771 ms suite-clock on a tree
# whose warm clock is an order of magnitude smaller
# (dev/benchmarks/provenance-removal-355565c2/README.md), because a
# dependency-profile change had invalidated every artifact. Reporting the build
# as its own step keeps that cost visible in gate evidence rather than hiding
# it.
#
# `cargo nextest run --no-run` links every test binary the `test` substep then
# executes, and with it the library rlibs and dependencies `cargo test --doc`
# links against: measured on this workspace, the doctest substep compiles
# nothing after this step, and repeating this step on a warm target is a no-op.
# Cargo rejects `cargo test --doc --no-run` ("can't skip running doc tests with
# --no-run") and exposes no stable way to precompile doctests, so rustdoc's own
# compilation of each doc fence has no prebuild and stays inside the clock,
# measured as part of the doctest substep.
run_step suite-build "${NICE_PREFIX[@]}" cargo nextest run --workspace --no-run

# The named suite clock is deliberately narrower than the gate duration. It
# starts immediately before the pinned workspace suite and ends after the
# separately reported doctest substep, leaving lock acquisition, preflight,
# formatting, linting, and the suite build above outside it.
suite_clock_started_ms=$(epoch_milliseconds)
run_step test    "${NICE_PREFIX[@]}" cargo nextest run --workspace
run_step doctest "${NICE_PREFIX[@]}" cargo test --doc --workspace
suite_clock_ms=$(( $(epoch_milliseconds) - suite_clock_started_ms ))
summary+="  ✓ suite-clock: ${suite_clock_ms} ms"$'\n'

# Build-footprint budget enforcement (jit:3f73423b). Runs AFTER the test step so
# its `cargo metadata` and `cargo test --workspace --no-run --message-format=json`
# reuse this run's warm target directory rather than triggering a second cold
# build (REQ-05): metadata performs no build, and --no-run only re-resolves and
# reports the test executables the `test` step already linked. The checker
# derives the integration-target count and unique active-executable bytes from
# that Cargo output, and asserts the debug-profile, gate-incremental, and
# dependency-feature policies against the committed manifests and this script.
# It is pointed at the workspace this run compiled rather than left to self-locate
# from its own path, so the budget verdict describes the same tree the steps above
# judged. CARGO_INCREMENTAL=0 (exported above) is inherited, so its warm --no-run
# leaves no incremental state for the check below.
#
# The same step enforces the suite budget (jit:94d85bf1) by handing the checker
# the suite clock measured above as `--test-suite-ms`. The threshold itself is
# the checker's own MAX_TEST_SUITE_SECONDS, declared once beside the artifact
# budgets; this script supplies the measurement, never a second copy of the
# limit. Without the argument the checker skips its duration check, so the
# budget would be declared and never enforced.
CARGO_CI_SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
run_budget_check() {
  local root
  root=$(gate_workspace_root) || return 1
  if [ -z "$root" ]; then
    echo "could not resolve this run's Cargo workspace root" >&2
    return 1
  fi
  "${NICE_PREFIX[@]}" "$CARGO_CI_SCRIPT_DIR/rust-build-budget.sh" \
    --root "$root" --test-suite-ms "$suite_clock_ms"
}
run_step budget run_budget_check

# REQ-04 (jit:57d0eb79): fail the gate itself if the compilation steps above
# left behind incremental state, rather than trusting that CARGO_INCREMENTAL=0
# held.
run_step incremental-state check_no_incremental_state

echo "$summary"

if [ "$failed" -ne 0 ]; then
  exit 1
fi
