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
# The run owns and removes its temporary directory: every Cargo, rustc and test
# process, the wrapper probe and `--cargo` included, runs with TMPDIR set to a
# directory made inside the caller's TMPDIR, and the directory goes on every
# exit, including a trapped INT, TERM or HUP. A compiler-wrapper daemon the
# run starts keeps the caller's TMPDIR (scripts/caller-tmpdir-wrapper.sh).
#
# The run takes no host-wide lock: concurrent runs on different worktrees share
# the host's cargo jobserver pool and keep every test within its per-test limit
# (dev/benchmarks/cargo-ci-concurrency-fba8d478/README.md). The pool exists only
# on this host; a host without it runs concurrent gates at its own risk.
#
# Exit codes:
#   0 — all steps passed
#   1 — one or more steps failed on repository content (a verdict)
#   2 — environment problem: Cargo prerequisites are missing or mismatched
#  75 — infrastructure fault (EX_TEMPFAIL): a compiler wrapper Cargo would use
#       cannot run, so no step ran; or every failing step is
#       environment-sensitive and either a step ran out of disk space (ENOSPC)
#       or the host was loaded while the tests ran
#
# Exit 75 is the exec-checker convention that records an `error` gate run
# instead of `failed`. An `error` run never passes and is never reused through
# digest reuse, so such a failure is rerun rather than carried to another
# issue as a verdict. A step is environment-sensitive only when it is
#   - any step whose output reports ENOSPC ("No space left on device" or
#     os error 28) and no other failure: no test failing without ENOSPC in its
#     own output, no compiler error, rustfmt diff or budget violation beyond
#     the ENOSPC line. ENOSPC makes the run exit 75 whatever the load baseline,
#   - `test` with at least one nextest TIMEOUT result and no FAIL or ABORT
#     result: a test over its layer's per-test limit (.config/nextest.toml).
# Any other failing step (fmt, clippy, doctest, excluded-tests, a test FAIL, a
# budget violation, a reporter failure) makes the run a verdict.
# The load rule below applies to the TIMEOUT case only.
# "Loaded" means the 1-minute load average from /proc/loadavg, sampled before
# the run's first step, is at least LOAD_THRESHOLD_PER_CPU x `nproc`. The
# baseline predates the run's own fmt, clippy and build fan-out, so it measures
# the host alone; a later sample would measure this run. The threshold is the
# steward's quiet-host bound (load below about 4 on 24 CPUs). A test timeout
# on a quiet host is a real regression against the change that crossed it and
# exits 1. The classification line names the
# baseline sample, the threshold and the classification either way.
#
# `./scripts/cargo-ci.sh --classify <work-dir> <load1> <cpus> [failed-step...]`
# runs only the classifier over a directory holding the steps' `<step>.out`
# files and prints its diagnosis; scripts/cargo-ci-selftest.sh drives every exit
# path through it.
#
# `./scripts/cargo-ci.sh --cargo <args...>` applies the same real Cargo
# selection and guarded sccache setup, then runs only that focused Cargo
# command. Use this mode for compilation-heavy targeted checks; invoke the
# script without arguments only when the complete gate is intended.
#
# `./scripts/cargo-ci.sh --suite` runs only the suite steps: pinned nextest,
# doctests, and the excluded-tests check. CI uses it so CI and the gate share one
# definition of the suite.

readonly LOAD_THRESHOLD_PER_CPU="0.17"

# Steps that failed, in run order, and the pre-run load sample the classifier judges.
failed_steps=()
baseline_load1=""

# 1-minute load average, empty when the host exposes none.
sample_load1() {
  awk 'NR == 1 { print $1 }' /proc/loadavg 2>/dev/null || true
}

# The output reports the disk was full.
reports_enospc() {
  [ -f "$1" ] && grep -qE 'No space left on device|os error 28' "$1"
}

# Failure markers in a step's output that ENOSPC does not account for. A failed
# test (nextest FAIL or ABORT, doctest FAILED) is ENOSPC-caused only when it
# panicked and every panic message in its block reports ENOSPC; a message runs
# from the `panicked at` line to the `note:` line or the end of the block, so
# ENOSPC in captured stdout or elsewhere in the block does not count. Outside
# test blocks a rustc `error` line (except the per-crate summary lines), a
# rustfmt `Diff in` line, or a budget violation counts unless the line itself
# reports ENOSPC.
count_non_enospc_failures() {
  awk '
    function enospc(line) { return line ~ /No space left on device|os error 28/ }
    function end_panic() {
      if (in_panic) { panics[pcur]++; if (msg_hit) good[pcur]++ }
      in_panic = 0
    }
    /^[[:space:]]*(TRY [0-9]+ )?[A-Z]+ \[/ {
      end_panic(); cur = ""
      if ($0 ~ /^[[:space:]]*(FAIL|ABORT) \[/) {
        cur = $0
        sub(/^[[:space:]]*[A-Z]+ \[[^]]*\][[:space:]]*/, "", cur)
        failed[cur] = 1
      }
      next
    }
    /^---- .* stdout ----$/ { end_panic(); cur = $0; sub(/^---- /, "", cur); sub(/ stdout ----$/, "", cur); next }
    /^test .* \.\.\. FAILED$/ { name = $0; sub(/^test /, "", name); sub(/ \.\.\. FAILED$/, "", name); failed[name] = 1; next }
    /^(failures:|test result:)/ { end_panic(); cur = ""; next }
    cur != "" && /panicked at / {
      end_panic(); in_panic = 1; pcur = cur; msg_hit = 0
      if ($0 !~ /:[0-9]+:[0-9]+:$/) msg_hit = enospc($0)
      next
    }
    cur != "" && /^(note:|--- )/ { end_panic(); next }
    cur != "" { if (in_panic && enospc($0)) msg_hit = 1; next }
    enospc($0) { next }
    /^error(\[[A-Za-z0-9]+\])?:/ && !/could not compile|aborting due to|previous error/ { n++; next }
    /^Diff in / || /^  - / { n++ }
    END {
      end_panic()
      for (name in failed) if (!(name in panics) || good[name] != panics[name]) n++
      print n + 0
    }
  ' "$1"
}

# The step failed because the disk was full: its output reports ENOSPC and
# carries no other failure marker.
step_failed_by_enospc() {
  local out="$WORK/$1.out"
  reports_enospc "$out" && [ "$(count_non_enospc_failures "$out")" -eq 0 ]
}

# nextest reported a TIMEOUT and nothing that is a verdict on the test itself.
test_failed_only_by_timeout() {
  grep -qE '^[[:space:]]*TIMEOUT \[' "$1" &&
    ! grep -qE '^[[:space:]]*(FAIL|ABORT) \[' "$1"
}

# Judges the failed steps in $failed_steps against the outputs in $WORK and the
# baseline load in $baseline_load1 over $cpu_count CPUs; prints the diagnosis and returns
# the exit status the run takes: 75 for an infrastructure fault, 1 for a verdict.
# ENOSPC steps need no load; the other environment-sensitive steps do.
classify_failure() {
  local step threshold
  local enospc_steps=() load_steps=()
  for step in "${failed_steps[@]}"; do
    if step_failed_by_enospc "$step"; then
      enospc_steps+=("$step")
      continue
    fi
    case "$step" in
      test) test_failed_only_by_timeout "$WORK/test.out" ;;
      *) false ;;
    esac || {
      echo "cargo-ci: verdict (exit 1): step '$step' failed on repository content"
      return 1
    }
    load_steps+=("$step")
  done
  if [ "${#load_steps[@]}" -eq 0 ]; then
    echo "cargo-ci: infrastructure fault (exit 75): failing steps (${failed_steps[*]}) ran out of disk space (ENOSPC: No space left on device); free space and rerun"
    return 75
  fi
  threshold=$(awk -v n="$cpu_count" -v r="$LOAD_THRESHOLD_PER_CPU" 'BEGIN { printf "%.2f", n * r }')
  if [ -n "$baseline_load1" ] &&
    awk -v l="$baseline_load1" -v t="$threshold" 'BEGIN { exit !(l + 0 >= t + 0) }'; then
    echo "cargo-ci: infrastructure fault (exit 75): failing steps (${failed_steps[*]}) are environment-sensitive${enospc_steps[*]:+ (ENOSPC: ${enospc_steps[*]})}; baseline load1=${baseline_load1} >= threshold ${threshold} (${LOAD_THRESHOLD_PER_CPU} x ${cpu_count} CPUs); rerun when the host is quiet"
    return 75
  fi
  echo "cargo-ci: verdict (exit 1): failing steps (${load_steps[*]}) are environment-sensitive but baseline load1=${baseline_load1:-unknown} < threshold ${threshold} (${LOAD_THRESHOLD_PER_CPU} x ${cpu_count} CPUs); a quiet-host failure is a real regression to investigate"
  return 1
}

if [ "${1:-}" = "--classify" ]; then
  [ "$#" -ge 4 ] || { echo "usage: cargo-ci.sh --classify <work-dir> <load1> <cpus> [failed-step...]" >&2; exit 2; }
  WORK="$2" baseline_load1="$3" cpu_count="$4"
  shift 4
  failed_steps=("$@")
  rc=0
  classify_failure || rc=$?
  exit "$rc"
fi

# The run owns one directory inside the caller's temporary directory and every
# Cargo, rustc and test process below sees it as TMPDIR, so what a killed test
# leaves behind dies with the run. The caller's TMPDIR is saved first: the
# compiler-cache wrapper restores it, because a daemon outlives the run.
export CARGO_CI_CALLER_TMPDIR="${TMPDIR:-/tmp}"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
export TMPDIR="$WORK/tmp"
mkdir "$TMPDIR"

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
# Whichever wrapper runs goes through one script that restores the caller's
# TMPDIR, because a daemon it starts or restarts mid-run outlives the run's own.
# Its stable path keeps Cargo fingerprints stable across runs.
unset CARGO_CI_CALLER_RUSTC_WRAPPER
if [ -n "${RUSTC_WRAPPER:-}" ]; then
  export CARGO_CI_CALLER_RUSTC_WRAPPER="$RUSTC_WRAPPER"
fi
if [ -z "${CARGO_CI_NO_SCCACHE:-}" ] && [ -z "${RUSTC_WRAPPER:-}" ] && command -v sccache >/dev/null 2>&1; then
  RUSTC_WRAPPER=sccache
fi
if [ -n "${RUSTC_WRAPPER:-}" ]; then
  RUSTC_WRAPPER="$(cd "$(dirname "$0")" && pwd)/caller-tmpdir-wrapper.sh"
  export RUSTC_WRAPPER
fi

# A wrapper that cannot compile would fail every compiling step as if on repository
# content, so try one first. Cargo compiles the probe crate from this directory,
# so it resolves every wrapper itself: RUSTC_WRAPPER, RUSTC_WORKSPACE_WRAPPER,
# their CARGO_BUILD_* forms and `build.*` config. CARGO_INCREMENTAL=0 keeps the
# compile cacheable, because sccache passes an incremental one through without
# reaching the temp directory a cacheable compile needs. Empty wrapper variables
# override every other source, so a failure that outlives them is no wrapper's
# and the steps judge it. Output goes to a file because a daemon the wrapper
# starts could hold a pipe open.
ensure_compiler_wrapper_runs() {
  local probe wrapper
  probe=$(mktemp -d)
  mkdir "$probe/src"
  : >"$probe/src/lib.rs"
  printf '[workspace]\n\n[package]\nname = "cargo-ci-wrapper-probe"\nversion = "0.0.0"\nedition = "2021"\n' \
    >"$probe/Cargo.toml"
  local -a check=(cargo check --verbose --manifest-path "$probe/Cargo.toml" --target-dir "$probe/target")
  if ! CARGO_INCREMENTAL=0 "${check[@]}" >"$probe/out" 2>&1 &&
    RUSTC_WRAPPER='' RUSTC_WORKSPACE_WRAPPER='' CARGO_INCREMENTAL=0 "${check[@]}" >/dev/null 2>&1; then
    # Cargo's failure quotes the invocation; the wrappers are the words before rustc.
    wrapper=$(grep -oE "(process didn't exit successfully: |could not execute process )\`[^\`]*" "$probe/out" |
      tail -1 | sed "s/^[^\`]*\`//" |
      awk '{ for (i = 1; i <= NF && $i !~ /(^|\/)rustc$/; i++) printf "%s%s", (i > 1 ? " " : ""), $i }' || true)
    [ -z "${RUSTC_WRAPPER:-}" ] || [ "$wrapper" != "$RUSTC_WRAPPER" ] ||
      wrapper="${CARGO_CI_CALLER_RUSTC_WRAPPER:-sccache}"
    echo "cargo-ci: infrastructure fault (exit 75): compiler wrapper '$wrapper' cannot compile an empty crate; no step ran"
    tail -20 "$probe/out"
    echo "       Repair the wrapper and retry: a shared daemon may need stopping"
    echo "       (e.g. sccache --stop-server), and CARGO_CI_NO_SCCACHE=1 bypasses sccache."
    rm -rf "$probe"
    exit 75
  fi
  rm -rf "$probe"
}

ensure_compiler_wrapper_runs

if [ "${1:-}" = "--cargo" ]; then
  shift
  cargo_status=0
  cargo "$@" || cargo_status=$?
  exit "$cargo_status"
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

# Cargo uses the workspace profile settings below: dev builds remain
# incremental, while test builds disable incremental state in Cargo.toml.

# This run's own scratch state (jit:0708d692): the Cargo artifact stream
# describing what this run built, so the budget step judges what this run
# produced rather than what it found on the machine.
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
    suite-build | excluded-tests)
      # These steps end their output with the one observation they made — how
      # long the build took, what the run held out of the default suite. The
      # persisted summary carries that observation rather than a bare "ok", so
      # gate evidence records the measurement and not just the verdict
      # (jit:0708d692).
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
      failed_steps+=("$name")
    fi
  else
    local rc=$?
    ended_ms=$(epoch_milliseconds)
    elapsed_ms=$((ended_ms - started_ms))
    summary+="  ✗ $name: FAILED (exit $rc) (${elapsed_ms} ms)"$'\n'
    summarize_fail "$name"
    failed=1
    failed_steps+=("$name")
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

# REQ-02 (jit:a36b91ff, @/invariant/no-ignored-tests): judge whether the run
# itself held out any suite tests or doctests. The reporter counts are the
# authoritative measurement because they describe what this run actually
# skipped; searching Rust sources would only be a proxy and would also match
# attributes or fences that do not hold a test out of the default run.
# nextest decides what it skips when it lists the tests, so a run fail-fast
# stopped ("N/M tests run") still reports the skipped count.
check_no_excluded_tests() {
  local reporter skipped ignored

  reporter=$(grep -E 'Summary .* [0-9]+(/[0-9]+)? tests run:' "$WORK/test.out" | tail -1 || true)
  skipped=$(grep -oP '\K[0-9]+(?= skipped)' <<<"$reporter" | tail -1 || true)
  if [ -z "$reporter" ] || ! grep -qP '[0-9]+ passed' <<<"$reporter"; then
    echo "cargo-ci: reporter-contract failure: could not parse pinned nextest success summary for excluded-test measurement" >&2
    return 1
  fi
  skipped=${skipped:-0}

  ignored=$(grep -oP '\K[0-9]+(?= ignored)' "$WORK/doctest.out" \
    | awk '{s+=$1} END {print s+0}')

  if [ "$skipped" -eq 0 ] && [ "$ignored" -eq 0 ]; then
    echo "excluded-tests: nextest skipped=0, doctest ignored=0"
    return 0
  fi

  echo "excluded-tests: nextest skipped=$skipped test(s); doctest ignored=$ignored test(s)"
  if [ "$skipped" -ne 0 ]; then
    echo "nextest tests held out of the default suite:"
    cargo nextest list --workspace --run-ignored ignored-only || return 1
  fi
  if [ "$ignored" -ne 0 ]; then
    echo "doctests held out of the default suite:"
    grep -E '^test .* ignored$' "$WORK/doctest.out" || true
  fi
  return 1
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

# Host load before this run's first step; the classifier judges it.
baseline_load1=$(sample_load1)

# Run all steps (continue through failures so every problem is reported, unlike
# a short-circuiting `&&` chain). Same checks the inline gate ran.
if [ "${1:-}" != "--suite" ]; then
  run_step fmt    "${NICE_PREFIX[@]}" cargo fmt --all -- --check
  run_step clippy "${NICE_PREFIX[@]}" cargo clippy --workspace --all-targets -- -D warnings
fi

# Build the product binaries the budget checker inspects and capture the Cargo
# artifact stream naming this run's test executables, so the budget step judges
# what this run built rather than what it found on the machine.
build_suite_artifacts() {
  local built_ms
  built_ms=$(epoch_milliseconds)
  "${NICE_PREFIX[@]}" cargo build --workspace --bins || return 1
  "${NICE_PREFIX[@]}" cargo test --workspace --no-run --message-format=json \
    >"$SUITE_ARTIFACTS_JSON" || return 1
  echo "built in $(( $(epoch_milliseconds) - built_ms )) ms"
}
if [ "${1:-}" != "--suite" ]; then
  run_step suite-build build_suite_artifacts
fi

run_step test    "${NICE_PREFIX[@]}" cargo nextest run --workspace
run_step doctest "${NICE_PREFIX[@]}" cargo test --doc --workspace

# This judges the run's own skipped/ignored counts, because those reporter
# measurements answer what the suite actually held out rather than what a
# source search merely appears to declare (jit:a36b91ff,
# @/invariant/no-ignored-tests).
run_step excluded-tests check_no_excluded_tests

# Build-footprint budget enforcement (jit:3f73423b). The checker derives the
# integration-target count from `cargo metadata` (which performs no build) and
# the unique active-executable bytes from the artifact stream the suite-build
# step captured, handed over as --artifacts-json: the budget then describes
# exactly the executables this run built and ran, and no second Cargo
# build is provoked here at all. It asserts the debug-profile and
# dependency-feature policies against the committed manifests. It is pointed at
# the workspace this run compiled rather than left to
# self-locate from its own path, so the budget verdict describes the same tree
# the steps above judged.
CARGO_CI_SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
run_budget_check() {
  local root
  root=$(gate_workspace_root) || return 1
  if [ -z "$root" ]; then
    echo "could not resolve this run's Cargo workspace root" >&2
    return 1
  fi
  "${NICE_PREFIX[@]}" "$CARGO_CI_SCRIPT_DIR/rust-build-budget.sh" \
    --root "$root" --artifacts-json "$SUITE_ARTIFACTS_JSON"
}
if [ "${1:-}" != "--suite" ]; then
  run_step budget run_budget_check
fi

echo "$summary"

if [ "$failed" -ne 0 ]; then
  cpu_count=$(nproc 2>/dev/null || echo 1)
  rc=0
  classify_failure || rc=$?
  exit "$rc"
fi
