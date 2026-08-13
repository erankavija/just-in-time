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
# accumulated without bound across gate runs (the measurement behind this
# policy is placed under "Provenance" in
# dev/benchmarks/rust-build-budgets/README.md). The
# `incremental-state` step below turns "should be disabled" into a checked
# fact rather than an assumption.
export CARGO_INCREMENTAL=0

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# This run's own scratch state (jit:0708d692): the incremental-state snapshot
# taken before the first compilation, and the Cargo artifact stream describing
# what this run built. Both exist so a step judges what this run produced rather
# than what it found on the machine.
INCREMENTAL_BASELINE="$WORK/incremental-baseline"
SUITE_ARTIFACTS_JSON="$WORK/suite-artifacts.json"

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
    suite-build | incremental-baseline | incremental-state)
      # These steps end their output with the one observation they made — what
      # was built and warmed, what incremental state was present, what this run
      # added to it. The persisted summary carries that observation rather than
      # a bare "ok", so gate evidence records the measurement and not just the
      # verdict (jit:0708d692).
      tail -1 "$WORK/$name.out"
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
    budget)
      echo "--- $name diagnostics ---"
      tail -40 "$WORK/$name.out"
      # REQ-05 (jit:0708d692): name the span the failing measurement was taken
      # over. The costs a cold target pays are compilation and first-touch
      # executable I/O; the suite-build step pays both before the clock starts
      # and reports what it did, so quoting that report here lets a reader
      # separate a slower suite from a colder machine without re-running the
      # gate and comparing numbers.
      echo "--- suite-clock scope ---"
      echo "suite-clock ${suite_clock_ms:-unmeasured} ms spans the test and doctest substeps only."
      echo "Excluded, and reported by the suite-build step: $(tail -1 "$WORK/suite-build.out")."
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

# REQ-04 (jit:57d0eb79): CARGO_INCREMENTAL=0 above is the mechanism; the paired
# steps below are the deterministic check that it held.
#
# REQ-03 (jit:0708d692): the check is a comparison, not an emptiness assertion.
# The target directory is shared with everything else that compiles this
# checkout — an editor's rust-analyzer writes `<target>/debug/incremental`
# continuously and repopulates it within seconds of being cleared — so an
# occupied directory is a fact about the machine and says nothing about the tree
# under judgement. Snapshotting before the first compilation and again after the
# last one attributes each entry to the run that created it, so this gate fails
# only on incremental state its own compilation produced.
#
# An entry is a path at most two levels below an `incremental` directory. rustc
# writes one session directory per compilation: under a per-crate directory
# (`incremental/<crate>-<hash>/s-<session>`) when Cargo enables incremental
# compilation, and directly under the directory named by an explicit
# `-Cincremental=DIR`. Both depths therefore name a unit that one compilation
# creates, while deeper session internals are excluded — they churn under a
# concurrent writer without naming state that was not already there.
#
# Suites that spawn scratch builds into their own throwaway target dirs
# (scratch_build) are out of scope by construction: only the directory
# `gate_target_dir` resolves is walked, never a suite's private scratch dir.
incremental_entries() {
  find "$1" -path '*/incremental/*' -not -path '*/incremental/*/*/*' -print 2>/dev/null \
    | LC_ALL=C sort
}

# This run's target directory, or a diagnostic and a failure when Cargo cannot
# name one — in which case neither incremental step can judge anything.
resolved_target_dir() {
  local target_dir
  target_dir=$(gate_target_dir) || return 1
  if [ -z "$target_dir" ]; then
    echo "could not resolve this run's Cargo target directory" >&2
    return 1
  fi
  printf '%s\n' "$target_dir"
}

capture_incremental_baseline() {
  local target_dir
  target_dir=$(resolved_target_dir) || return 1
  incremental_entries "$target_dir" >"$INCREMENTAL_BASELINE"
  echo "$(wc -l <"$INCREMENTAL_BASELINE") entries already present under $target_dir, none of them this run's"
}

check_incremental_state_from_this_run() {
  local -a added=()
  local target_dir current baseline_count
  target_dir=$(resolved_target_dir) || return 1
  if [ ! -r "$INCREMENTAL_BASELINE" ]; then
    echo "no pre-compilation baseline was captured, so incremental state cannot be attributed to this run" >&2
    return 1
  fi
  current="$WORK/incremental-current"
  incremental_entries "$target_dir" >"$current"
  baseline_count=$(wc -l <"$INCREMENTAL_BASELINE")
  mapfile -t added < <(comm -13 "$INCREMENTAL_BASELINE" "$current")
  if [ "${#added[@]}" -ne 0 ]; then
    {
      echo "this run's own compilation created incremental state under $target_dir."
      echo "  observed: ${#added[@]} incremental entries that the incremental-baseline step did not see before any compilation ran:"
      printf '    %s\n' "${added[@]}"
      echo "  compared against: the $baseline_count entries already present when this run started, which another process wrote and this check ignores."
      echo "  CARGO_INCREMENTAL=0 did not hold for every compilation step. This is a policy regression in the tree under judgement, not a condition of the machine."
    } >&2
    return 1
  fi
  echo "no incremental state created by this run ($baseline_count pre-existing entries ignored)"
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
# Record what is already there before paying for any compilation, so the final
# incremental-state step can tell this run's output from the machine's. The only
# failure this step can report is a target directory Cargo will not name, which
# leaves the policy unjudgeable for the whole run — so it stops here rather than
# after the expensive steps.
run_step incremental-baseline capture_incremental_baseline
if [ "$failed" -ne 0 ]; then
  echo "$summary"
  exit 1
fi

run_step fmt    "${NICE_PREFIX[@]}" cargo fmt --all -- --check
run_step clippy "${NICE_PREFIX[@]}" cargo clippy --workspace --all-targets -- -D warnings

# Put the measured suite into the state the clock assumes, before the clock
# starts (jit:94d85bf1, jit:0708d692): compiled, and read into the page cache.
#
# The budget enforced below is defined over an already-built target, so
# compilation paid inside the clock would fail the gate on every cold one —
# every fresh clone and every CI runner — with nothing wrong in the tree. The
# recorded cargo-ci run for jit:6d10e5d4 at commit e2da055a reports a 239,771 ms
# suite-clock on a tree whose warm clock is an order of magnitude smaller
# (dev/benchmarks/provenance-removal-355565c2/README.md), because a
# dependency-profile change had invalidated every artifact.
#
# Compiling here was not enough on its own: measured at one commit on one host,
# the same passing suite clocked 57,019 ms on a fresh target against 22,541 ms
# on a warm one, failing and passing one budget for a reason the tree had no
# part in (dev/benchmarks/cold-warm-verdict-0708d692/README.md). Most of that
# was compilation this step had not covered, because the suite compiled from
# inside its own clock — see scripts/setup-recorded-failure-corpus.sh, whose
# Cargo package selection now matches the build below.
#
# What remains here is the artifacts themselves: nothing has read the
# executables this step links, so the `test` substep would fault them in while
# the clock runs. Reading them back costs this step what the page cache no
# longer holds — 3,430 ms with those executables evicted, 70 ms when the build
# above just wrote them — and keeps that cost out of the measured span.
# Reporting it as this step's own also makes the machine's state legible in gate
# evidence: the warm figure says whether this run found a resident target or a
# stale one.
#
# `cargo nextest run --no-run` links every test binary the `test` substep then
# executes, and with it the library rlibs and dependencies `cargo test --doc`
# links against: measured on this workspace, the doctest substep compiles
# nothing after this step, and repeating this step on a warm target is a no-op.
# Cargo rejects `cargo test --doc --no-run` ("can't skip running doc tests with
# --no-run") and exposes no stable way to precompile doctests, so rustdoc's own
# compilation of each doc fence has no prebuild and stays inside the clock,
# measured as part of the doctest substep. The same cold/warm measurement shows
# that substep pays no first-touch cost of its own (4,555 ms cold against
# 4,552 ms warm), so what this step warms is the executables the `test` substep
# runs, not every artifact on the way to them.
#
# The stream naming those executables is the one the budget step below needs
# anyway; it is captured once here and handed to that step as --artifacts-json,
# so one Cargo invocation produces one description of what this run built and
# both steps read it. That invocation also links the targets `cargo test` builds
# and `cargo nextest run --no-run` does not, keeping their compilation outside
# the clock too.
prepare_measured_suite() {
  local -a executables=() readable=()
  local exe size bytes=0 built_ms warm_started_ms
  built_ms=$(epoch_milliseconds)
  "${NICE_PREFIX[@]}" cargo nextest run --workspace --no-run || return 1
  "${NICE_PREFIX[@]}" cargo test --workspace --no-run --message-format=json \
    >"$SUITE_ARTIFACTS_JSON" || return 1
  built_ms=$(( $(epoch_milliseconds) - built_ms ))

  warm_started_ms=$(epoch_milliseconds)
  mapfile -t executables < <(
    jq -r 'select(.reason == "compiler-artifact" and .executable != null) | .executable' \
      "$SUITE_ARTIFACTS_JSON" | LC_ALL=C sort -u
  )
  for exe in "${executables[@]}"; do
    size=$(stat -c %s "$exe" 2>/dev/null) || continue
    readable+=("$exe")
    bytes=$((bytes + size))
  done
  if [ "${#readable[@]}" -eq 0 ]; then
    echo "no readable executables in this run's Cargo artifact stream (expected at least one to warm)" >&2
    return 1
  fi
  "${NICE_PREFIX[@]}" cat -- "${readable[@]}" >/dev/null || return 1

  echo "built in ${built_ms} ms, warmed ${#readable[@]} executables (${bytes} bytes) in $(( $(epoch_milliseconds) - warm_started_ms )) ms"
}
run_step suite-build prepare_measured_suite

# The named suite clock is deliberately narrower than the gate duration. It
# starts immediately before the pinned workspace suite and ends after the
# separately reported doctest substep, leaving lock acquisition, preflight,
# formatting, linting, and the suite build above outside it.
suite_clock_started_ms=$(epoch_milliseconds)
run_step test    "${NICE_PREFIX[@]}" cargo nextest run --workspace
run_step doctest "${NICE_PREFIX[@]}" cargo test --doc --workspace
suite_clock_ms=$(( $(epoch_milliseconds) - suite_clock_started_ms ))
summary+="  ✓ suite-clock: ${suite_clock_ms} ms"$'\n'

# Build-footprint budget enforcement (jit:3f73423b). The checker derives the
# integration-target count from `cargo metadata` (which performs no build) and
# the unique active-executable bytes from the artifact stream the suite-build
# step captured, handed over as --artifacts-json: the budget then describes
# exactly the executables this run built, warmed, and ran, and no second Cargo
# build is provoked here at all. It asserts the debug-profile, gate-incremental,
# and dependency-feature policies against the committed manifests and this
# script. It is pointed at the workspace this run compiled rather than left to
# self-locate from its own path, so the budget verdict describes the same tree
# the steps above judged.
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
    --root "$root" --artifacts-json "$SUITE_ARTIFACTS_JSON" --test-suite-ms "$suite_clock_ms"
}
run_step budget run_budget_check

# REQ-04 (jit:57d0eb79): fail the gate itself if the compilation steps above
# left behind incremental state, rather than trusting that CARGO_INCREMENTAL=0
# held. What counts as "left behind" is what was not there before those steps
# ran, per the baseline the first step recorded (REQ-03, jit:0708d692).
run_step incremental-state check_incremental_state_from_this_run

echo "$summary"

if [ "$failed" -ne 0 ]; then
  exit 1
fi
