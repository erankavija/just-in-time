#!/usr/bin/env bash
# The scenario fixtures and assertion predicates below are invoked indirectly —
# by name through run_scenario, as a command through check, and via the EXIT
# trap — which shellcheck cannot follow.
# shellcheck disable=SC2329
# NB: NOT `set -e` — this harness inspects child exit codes on purpose.
set -uo pipefail

# cargo-ci-selftest — can the gate that judges a merged tree still fail it?
#
# A textually clean merge can leave the mainline broken. One branch deletes a
# module file while another declares it; one branch changes a function's
# signature while another adds a caller of the old form. Neither side conflicts,
# and every per-issue gate already passed against a tree that predates the merge,
# so nothing but a check run on the merged tree observes the combination. That
# check is scripts/cargo-ci.sh, run on the merge result.
#
# Relying on it is only sound while it can still fail. A build-only check —
# `cargo build --workspace` — compiles neither test targets nor dev-dependencies,
# so a merge that breaks only test code passes it, and a merge that compiles but
# whose tests fail passes it too. This self-test seeds clean merges that break in
# each of those ways, runs the SHIPPED gate script against them (no copy of its
# logic lives here), and asserts what it reports:
#
#   healthy      a clean merge that builds and passes its tests -> step passes
#   resurrection a merge declaring a deleted module             -> step fails
#   signature    a merge whose #[cfg(test)] caller lost its arg -> step fails
#   stale-expect a merge that compiles, carrying a test that
#                asserts the pre-merge value                    -> step fails
#   ignored-test a merge holding out a test with #[ignore]       -> excluded-tests fails
#   ignored-doctest a merge holding out an ```ignore``` doctest  -> excluded-tests fails
#   over-limit   a test over its layer's per-test limit       -> step fails
#
# The exit-status classifier (0/1/75) is driven through `cargo-ci.sh --classify`
# over synthetic step output and load samples, one assertion per exit path. A
# compiler wrapper that cannot run exits 75 before any step, driven through
# wrappers this self-test owns. The build-lock wait of this gate and of the
# profiling and benchmark scripts is driven against a lock it owns, in a repository
# it owns.
#
# The signature and stale-expect scenarios also assert that `cargo build
# --workspace` SUCCEEDS on the same tree: that is the recorded reason a
# build-only merge guard was vacuous, kept here as a regression so the
# distinction cannot quietly be lost again.
#
# Cost: every fixture is a dependency-free two-module crate in a throwaway git
# repo with its own target directory. This workspace is never rebuilt.
#
# Exit codes:
#   0 — all assertions passed
#   1 — one or more assertions failed
#   2 — environment error (missing tooling)

for tool in git cargo jq python3 flock; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "selftest: '$tool' not on PATH" >&2
    exit 2
  }
done

here=$(cd "$(dirname "$0")" && pwd)
gate="$here/cargo-ci.sh"
[ -x "$gate" ] || {
  echo "selftest: gate script $gate not found or not executable" >&2
  exit 2
}

scratch=$(mktemp -d)
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT

# Fixtures select their own wrapper, so the host's cannot decide their outcome.
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER CARGO_BUILD_RUSTC_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER

fail=0
real_cargo_version=$(cargo --version)

# Records one assertion. The condition is a command so each call reads as the
# property it asserts rather than as a bare status code.
check() { # check <message> <command...>
  local msg="$1"
  shift
  if "$@"; then
    echo "PASS: $msg"
  else
    echo "FAIL: $msg"
    fail=1
  fi
}

# --- the gate's reported verdict ---------------------------------------------
# cargo-ci prints one summary line per step: "  ✓ <step>: ..." when the step
# passed and "  ✗ <step>: FAILED (exit N)" when it did not. `test` is the
# pinned-nextest step that compiles every target and runs the tests, so its line
# is the gate's verdict on the merged tree. The repository-specific steps
# (budget) cannot hold in a throwaway crate and are not what these
# assertions read.
readonly STEP_PASSED="  ✓ "
readonly STEP_FAILED="  ✗ "

step_passed() { grep -qF "$STEP_PASSED$2:" "$1"; }
step_failed() { grep -qF "$STEP_FAILED$2:" "$1"; }

# Step timing is evidence, not a performance assertion: accept any non-negative
# integer-millisecond value, while requiring every stable step summary to carry
# one. Real elapsed values vary with host load, so the contract deliberately
# avoids fixed-duration expectations.
all_step_summaries_have_integer_milliseconds() {
  local summaries
  summaries=$(grep -E '^  [✓✗] [a-z-]+: ' "$1")
  [ -n "$summaries" ] &&
    ! grep -Ev '^  [✓✗] [a-z-]+: .+ \([0-9]+ ms\)$' <<<"$summaries"
}

# Real reporter evidence, not just the stable step prefix. The healthy fixture
# has exactly four nextest-run tests. The stale-expect fixture has exactly three
# tests, one of which fails at runtime. These predicates ensure cargo-ci's
# concise reporter preserves both totals and the failing semantic test identity.
nextest_success_reported() {
  grep -qF "$STEP_PASSED"'test: 4 passed, 0 failed, 0 skipped' "$1"
}

nextest_runtime_failure_reported() {
  grep -qF -- '--- test failures ---' "$1" &&
    grep -Eq 'FAIL .*test_alpha_value_is_the_base_value' "$1" &&
    grep -Eq 'Summary .*3 tests run: 2 passed, 1 failed' "$1"
}

# The gate must reject any reporter version other than the provisioned pin
# before it reaches fmt or another gate step. A cargo shim answers the real
# cargo's version, captured up front because a PATH-scanning host cargo would
# resolve back to this shim, and injects an older nextest version; any Cargo
# invocation after the wrapper probe's `check` is an assertion failure in the
# fixture itself.
test_wrong_nextest_version_fails_fast() {
  local fixture="$scratch/wrong-nextest-version"
  local fake_bin="$fixture/bin"
  local out="$fixture/gate.out"
  local unexpected="$fixture/unexpected-cargo"
  local rc
  mkdir -p "$fake_bin" "$fixture/repo"
  cat >"$fake_bin/cargo" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "--version" ]; then
  echo "$SELFTEST_CARGO_VERSION"
  exit 0
fi
if [ "${1:-}" = "nextest" ] && [ "${2:-}" = "--version" ]; then
  echo "cargo-nextest 0.9.132 (injected selftest fixture)"
  exit 0
fi
if [ "${1:-}" = "check" ]; then
  exit 0
fi
printf '%s\n' "$*" >"$SELFTEST_UNEXPECTED_CARGO"
exit 97
EOF
  chmod +x "$fake_bin/cargo"

  (
    cd "$fixture/repo" || exit 3
    PATH="$fake_bin:$PATH" \
      SELFTEST_CARGO_VERSION="$real_cargo_version" \
      SELFTEST_UNEXPECTED_CARGO="$unexpected" \
      CARGO_CI_NO_LOCK=1 \
      CARGO_CI_NO_SCCACHE=1 \
      "$gate"
  ) >"$out" 2>&1
  rc=$?

  check "wrong nextest version: the gate exits with an environment error" \
    test "$rc" -eq 2
  check "wrong nextest version: the error names the required and observed versions" \
    grep -qF "requires cargo-nextest 0.9.133" "$out"
  check "wrong nextest version: no gate step or later Cargo command runs" \
    test ! -e "$unexpected"
}

# Runs the gate in --suite mode under a Cargo shim whose nextest run prints
# SELFTEST_SUITE_OUTPUT and exits with the given status; sets rc, and leaves
# the gate output in $suite_out and the Cargo commands it ran in $suite_log.
suite_gate() { # suite_gate <nextest status> [nextest output]
  local fixture="$scratch/suite-mode"
  suite_log="$fixture/cargo.log" suite_out="$fixture/out"
  if [ ! -x "$fixture/bin/cargo" ]; then
    mkdir -p "$fixture/bin" "$fixture/repo"
    cat >"$fixture/bin/cargo" <<'EOF2'
#!/usr/bin/env bash
case "$*" in
  --version) echo "$SELFTEST_CARGO_VERSION" ;;
  "nextest --version") echo "cargo-nextest 0.9.133 (selftest fixture)" ;;
  check*) ;;
  "nextest run --workspace")
    echo "$*" >>"$SELFTEST_CARGO_LOG"
    printf '%s\n' "$SELFTEST_SUITE_OUTPUT"
    exit "$SELFTEST_SUITE_STATUS"
    ;;
  *)
    echo "$*" >>"$SELFTEST_CARGO_LOG"
    echo "test result: ok. 1 passed; 0 failed; 0 ignored"
    ;;
esac
EOF2
    chmod +x "$fixture/bin/cargo"
  fi
  : >"$suite_log"
  (
    cd "$fixture/repo" || exit 3
    PATH="$fixture/bin:$PATH" \
      SELFTEST_CARGO_VERSION="$real_cargo_version" \
      SELFTEST_CARGO_LOG="$suite_log" \
      SELFTEST_SUITE_STATUS="$1" \
      SELFTEST_SUITE_OUTPUT="${2:-Summary [1s] 1 tests run: 1 passed, 0 skipped}" \
      CARGO_CI_NO_LOCK=1 \
      CARGO_CI_NO_SCCACHE=1 \
      "$gate" --suite
  ) >"$suite_out" 2>&1
  rc=$?
}

# CI's entry point runs the gate's suite steps and nothing else, and a failing
# suite fails it.
test_suite_mode_runs_only_the_suite_steps() {
  local rc
  suite_gate 0
  check "suite mode: a passing suite exits 0" test "$rc" -eq 0
  check "suite mode: Cargo runs the pinned suite and the doctests, nothing else" \
    test "$(cat "$suite_log")" = "nextest run --workspace"$'\n'"test --doc --workspace"
  check "suite mode: the excluded-tests step judges the run" \
    grep -qF "✓ excluded-tests:" "$suite_out"

  suite_gate 1
  check "suite mode: a failing suite fails the run" test "$rc" -eq 1
}

# Fail-fast stops nextest at its first TIMEOUT and reports a partial count
# ("N/M tests run"). The timeout is then the run's only failing step, so the
# classifier judges it by load; excluded-tests still reads the skipped count.
test_fail_fast_timeout_fails_only_the_test_step() {
  local rc
  suite_gate 100 "$TIMEOUT_LINE"$'\n''     Summary [   4.496s] 395/5179 tests run: 393 passed, 2 timed out, 0 skipped'
  check "fail-fast timeout: the test step fails" step_failed "$suite_out" test
  check "fail-fast timeout: the excluded-tests step reads the partial run's skipped count" \
    step_passed "$suite_out" excluded-tests
  check "fail-fast timeout: the classifier judges the test step alone" \
    grep -qF "failing steps (test)" "$suite_out"
}

# The real wrapper must distinguish lock acquisition from every child status.
test_build_lock_status_propagation() {
  local fixture="$scratch/build-lock" rc status
  mkdir -p "$fixture/bin"
  cat >"$fixture/bin/cargo" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "--version" ]; then
  echo "cargo 1.90.0 (selftest fixture)"
  exit 0
fi
if flock -n "$CARGO_CI_BUILD_LOCK" true; then
  echo "child ran without the build lock" >&2
  exit 98
fi
exit "$SELFTEST_CHILD_STATUS"
EOF
  chmod +x "$fixture/bin/cargo"
  for status in 0 1 2 75 255; do
    env -u CARGO_CI_LOCKED -u CARGO_CI_NO_LOCK \
      PATH="$fixture/bin:$PATH" CARGO_CI_NO_SCCACHE=1 \
      CARGO_CI_BUILD_LOCK="$fixture/build.lock" CARGO_CI_LOCK_TIMEOUT=0.01 \
      SELFTEST_CHILD_STATUS="$status" \
      "$gate" --cargo selftest >"$fixture/out" 2>&1
    rc=$?
    check "build lock: child status $status survives while the lock is held" test "$rc" -eq "$status"
    check "build lock: child status $status is not diagnosed as acquisition timeout" \
      test "$(grep -c 'timed out' "$fixture/out")" -eq 0
  done
  check "build lock: child exit releases the lock" flock -n "$fixture/build.lock" true
}

# A wrapper that cannot run is the environment, whichever source selects it and
# whichever entry point reaches it. Cargo here is real, because the probe compiles
# through it. The sccache stand-in answers version queries and stdin input and
# fails every compile, as a daemon with a deleted TMPDIR does.
test_unrunnable_compiler_wrapper() {
  local fixture="$scratch/wrapper" out="$scratch/wrapper/gate.out" rc
  local broken="$fixture/bin/sccache" missing="$fixture/missing-wrapper"
  local passthrough="$fixture/passthrough-wrapper"
  mkdir -p "$fixture/bin" "$fixture/cwd" "$fixture/configured/.cargo"
  cat >"$broken" <<'EOF'
#!/usr/bin/env bash
case " $* " in
  *" -vV "* | *" - "*) exec "$@" ;;
esac
echo "sccache: error: Failed to create temp dir" >&2
exit 254
EOF
  cat >"$passthrough" <<'EOF'
#!/usr/bin/env bash
exec "$@"
EOF
  chmod +x "$broken" "$passthrough"
  printf '[build]\nrustc-wrapper = "%s"\n' "$missing" >"$fixture/configured/.cargo/config.toml"

  gate_from() { # gate_from <cwd> <gate-args> [VAR=value ...]; sets rc
    local cwd="$1" args="$2"
    shift 2
    # shellcheck disable=SC2086 # <gate-args> is a word list
    (cd "$cwd" && env -u CARGO_CI_NO_SCCACHE PATH="$fixture/bin:$PATH" CARGO_CI_NO_LOCK=1 "$@" "$gate" $args) >"$out" 2>&1
    rc=$?
  }
  focused_gate_from() { # focused_gate_from <cwd> [VAR=value ...]; sets rc
    local cwd="$1"
    shift
    gate_from "$cwd" "--cargo --version" CARGO_CI_NO_SCCACHE=1 "$@"
  }
  no_cargo_command_ran() { ! grep -q '^cargo [0-9]' "$out"; }

  echo
  echo "== compiler wrapper: one that cannot run is an environment fault =="
  gate_from "$fixture/cwd" ""
  check "selected sccache fails compiles: the gate exits with the infrastructure status" \
    test "$rc" -eq 75
  check "selected sccache fails compiles: the diagnosis names the wrapper" \
    grep -qF "compiler wrapper 'sccache'" "$out"
  check "selected sccache fails compiles: the diagnosis carries the wrapper's error" \
    grep -qF "Failed to create temp dir" "$out"
  check "selected sccache fails compiles: no verdict on repository content" \
    test "$(grep -c 'repository content' "$out")" -eq 0
  check "selected sccache fails compiles: no gate step runs" \
    test "$(grep -cE '^  [✓✗] ' "$out")" -eq 0

  focused_gate_from "$fixture/cwd" RUSTC_WRAPPER="$missing"
  check "RUSTC_WRAPPER is missing: focused Cargo mode exits with the infrastructure status" \
    test "$rc" -eq 75
  check "RUSTC_WRAPPER is missing: the diagnosis names the wrapper" \
    grep -qF "compiler wrapper '$missing'" "$out"
  check "RUSTC_WRAPPER is missing: the Cargo command never runs" no_cargo_command_ran

  focused_gate_from "$fixture/cwd" RUSTC_WORKSPACE_WRAPPER="$broken"
  check "RUSTC_WORKSPACE_WRAPPER fails compiles: the gate exits with the infrastructure status" \
    test "$rc" -eq 75
  check "RUSTC_WORKSPACE_WRAPPER fails compiles: the diagnosis names the wrapper" \
    grep -qF "compiler wrapper '$broken'" "$out"

  focused_gate_from "$fixture/configured"
  check "build.rustc-wrapper is missing: the gate exits with the infrastructure status" \
    test "$rc" -eq 75
  check "build.rustc-wrapper is missing: the diagnosis names the wrapper" \
    grep -qF "compiler wrapper '$missing'" "$out"

  focused_gate_from "$fixture/cwd" RUSTC_WRAPPER="$passthrough" RUSTC_WORKSPACE_WRAPPER="$passthrough"
  check "healthy wrappers: the Cargo command runs" \
    test "$rc" -eq 0 -a "$(grep -c '^cargo [0-9]' "$out")" -eq 1

  focused_gate_from "$fixture/cwd" RUSTFLAGS=--selftest-invalid-flag
  check "a probe failure that outlives removing the wrappers is left to the steps" \
    test "$rc" -eq 0 -a "$(grep -c 'infrastructure fault' "$out")" -eq 0
}

# Each script that takes the build lock bounds and announces its wait, and
# no Cargo build starts without the lock. Every run happens in a repository this
# test owns with every output overridden, because the benchmarks create directories
# before they lock. The flock shim hands over at the script's first attempt, so a
# lock arriving after the script started is deterministic on either side of that
# attempt. A benchmark-rust-build.sh timeout before it first holds the lock keeps
# its samples; one after a finished sample reports the partial set; neither leaves
# a target directory.
test_script_build_lock_wait() {
  local fixture="$scratch/script-lock" repo="$scratch/script-lock/repo" bound=0.01
  local lock="$fixture/build.lock" cargo_ran="$fixture/cargo-ran"
  local first_attempt="$fixture/first-attempt" proceed="$fixture/proceed"
  local real_flock script name out rc lock_fd arrival_fd pid arrival_took_lock
  local clean_written rebuild_written
  real_flock=$(command -v flock)
  mkdir -p "$fixture/bin" "$fixture/shim"
  seed_base "$repo"
  cat >"$fixture/bin/cargo" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "--version" ]; then
  echo "cargo 1.90.0 (selftest fixture)"
  exit 0
fi
printf '%s\n' "$*" >"$SELFTEST_CARGO_RAN"
exit "${SELFTEST_CARGO_STATUS:-97}"
EOF
  printf '#!/usr/bin/env bash\n' >"$fixture/bin/rg"
  cat >"$fixture/shim/flock" <<'EOF'
#!/usr/bin/env bash
hand_over() {
  : >"$SELFTEST_FIRST_ATTEMPT"
  local waited=0
  while [ ! -e "$SELFTEST_PROCEED" ] && [ "$waited" -lt 3000 ]; do
    sleep 0.01
    waited=$((waited + 1))
  done
}
[ ! -e "$SELFTEST_FIRST_ATTEMPT" ] || exec "$SELFTEST_REAL_FLOCK" "$@"
case "$SELFTEST_LOCK_ARRIVES" in
  after-second-release)
    [ "${1:-}" = "-u" ] || exec "$SELFTEST_REAL_FLOCK" "$@"
    "$SELFTEST_REAL_FLOCK" "$@"
    if [ -e "$SELFTEST_FIRST_ATTEMPT.released" ]; then
      hand_over
    fi
    : >"$SELFTEST_FIRST_ATTEMPT.released"
    ;;
  before-attempt)
    hand_over
    exec "$SELFTEST_REAL_FLOCK" "$@"
    ;;
  after-attempt)
    rc=0
    "$SELFTEST_REAL_FLOCK" "$@" || rc=$?
    hand_over
    exit "$rc"
    ;;
esac
EOF
  chmod +x "$fixture/bin/cargo" "$fixture/bin/rg" "$fixture/shim/flock"

  run_script() { # run_script <script> <PATH-prefix> [VAR=value ...]
    local path_prefix="$2" script_path="$1"
    shift 2
    (
      cd "$repo" || exit 3
      env -u CARGO_CI_LOCKED -u CARGO_CI_NO_LOCK -u PROFILE_TEST_SUITE_LOCKED -u SESSION_BENCH_BIN \
        PATH="$path_prefix:$PATH" SELFTEST_CARGO_RAN="$cargo_ran" \
        CARGO_CI_BUILD_LOCK="$lock" CARGO_CI_LOCK_TIMEOUT="$bound" \
        PROFILE_TEST_SUITE_OUTPUT="$fixture/profile.json" \
        SESSION_BENCH_OUT_DIR="$fixture/session-out" SESSION_BENCH_WORK_DIR="$fixture/session-work" \
        SESSION_BENCH_TARGET_DIR="$fixture/session-target" \
        BENCH_OUT_DIR="$fixture/bench-out" BENCH_TARGET_BASE="$fixture/bench-target" \
        BENCH_PROBE_FILE=src/lib.rs "$@" "$script_path"
    ) >"$out" 2>&1
  }
  # Runs <script> until its first flock attempt, takes the lock if it is free, and
  # lets the script finish.
  run_with_arriving_lock() { # run_with_arriving_lock <script> <before-attempt|after-attempt|after-second-release> [VAR=value ...]
    local arriving_script="$1" arrival="$2"
    shift 2
    rm -f "$first_attempt" "$first_attempt.released" "$proceed"
    run_script "$arriving_script" "$fixture/shim:$fixture/bin" SELFTEST_LOCK_ARRIVES="$arrival" \
      SELFTEST_REAL_FLOCK="$real_flock" SELFTEST_FIRST_ATTEMPT="$first_attempt" \
      SELFTEST_PROCEED="$proceed" "$@" &
    pid=$!
    while [ ! -e "$first_attempt" ] && kill -0 "$pid" 2>/dev/null; do
      sleep 0.01
    done
    exec {arrival_fd}>"$lock"
    arrival_took_lock=no
    if flock -n "$arrival_fd"; then
      arrival_took_lock=yes
    fi
    : >"$proceed"
    wait "$pid"
    rc=$?
    exec {arrival_fd}>&-
  }
  wait_is_announced() { grep -F "waiting up to ${bound}s" "$out" | grep -qF "$lock"; }
  no_target_dir_remains() { ! compgen -G "$fixture/bench-target/target.*" >/dev/null; }

  echo
  echo "== script build lock: the wait is bounded, announced and builds nothing =="
  for script in cargo-ci.sh profile-test-suite.sh benchmark-session-cost.sh benchmark-rust-build.sh; do
    name=$script
    script="$here/$script"
    out="$fixture/$name.out"
    rm -f "$cargo_ran"
    if [ "$name" = benchmark-rust-build.sh ]; then
      mkdir -p "$fixture/bench-out/raw"
      printf '{"sample":1}\n' | tee "$fixture/seed" "$fixture/bench-out/raw/clean-samples.jsonl" \
        >"$fixture/bench-out/raw/rebuild-samples.jsonl"
    fi

    exec {lock_fd}>"$lock"
    flock "$lock_fd"
    run_script "$script" "$fixture/bin"
    rc=$?
    exec {lock_fd}>&-
    check "$name lock held throughout: the wait passing exits 2" test "$rc" -eq 2
    check "$name lock held throughout: the wait names the lock and the bound" wait_is_announced
    check "$name lock held throughout: the timeout names the lock and the bound" \
      grep -qF "timed out after ${bound}s waiting for $lock" "$out"
    check "$name lock held throughout: no Cargo build starts" test ! -e "$cargo_ran"
    check "$name lock held throughout: the repository is unchanged" \
      test -z "$(git -C "$repo" status --porcelain)"
    if [ "$name" = benchmark-rust-build.sh ]; then
      check "$name lock held throughout: the clean samples are unchanged" \
        cmp -s "$fixture/seed" "$fixture/bench-out/raw/clean-samples.jsonl"
      check "$name lock held throughout: the rebuild samples are unchanged" \
        cmp -s "$fixture/seed" "$fixture/bench-out/raw/rebuild-samples.jsonl"
      check "$name lock held throughout: no target directory remains" no_target_dir_remains
      check "$name lock held throughout: the run is not reported as partial" \
        test "$(grep -c 'partial' "$out")" -eq 0

      run_with_arriving_lock "$script" after-second-release SELFTEST_CARGO_STATUS=0 \
        BENCH_INVENTORY_SAMPLE=0
      clean_written=$(wc -l <"$fixture/bench-out/raw/clean-samples.jsonl")
      rebuild_written=$(wc -l <"$fixture/bench-out/raw/rebuild-samples.jsonl")
      check "$name lock taken after a finished sample: that sample is recorded" \
        test "$clean_written" -gt 0
      check "$name lock taken after a finished sample: the wait passing exits 2" test "$rc" -eq 2
      check "$name lock taken after a finished sample: the message counts the partial set" \
        grep -qE "partial.*\b$clean_written clean and $rebuild_written rebuild samples" "$out"
      check "$name lock taken after a finished sample: no target directory remains" no_target_dir_remains
      rm -f "$cargo_ran"
    fi

    run_with_arriving_lock "$script" before-attempt
    check "$name lock arrives after the start: the wait is announced" wait_is_announced
    check "$name lock arrives after the start: the wait passing exits 2" test "$rc" -eq 2
    check "$name lock arrives after the start: no Cargo build starts" test ! -e "$cargo_ran"

    run_with_arriving_lock "$script" after-attempt
    check "$name lock free at the first attempt: that attempt holds it, so no wait goes silent" \
      test "$arrival_took_lock" = no
    git -C "$repo" clean -fdxq
  done
}

# A run owns its temporary directory: every Cargo process it starts, the wrapper
# probe and the focused `--cargo` path included, sees a directory inside the
# caller's TMPDIR, and a file a test leaves there is gone once the run ends,
# whether the run passed or failed. A compiler-cache server a probe or a compile
# starts keeps the caller's TMPDIR. The caller's TMPDIR is a directory this test
# owns, so it also holds what the run leaves.
test_run_removes_its_temporary_directory() {
  local fixture="$scratch/tmpdir-owned" status
  mkdir -p "$fixture/bin" "$fixture/repo"
  cat >"$fixture/bin/cargo" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  --version) echo "$SELFTEST_CARGO_VERSION" ;;
  "nextest --version") echo "cargo-nextest 0.9.133 (selftest fixture)" ;;
  check*)
    echo "probe $TMPDIR" >>"$SELFTEST_SEEN"
    "$RUSTC_WRAPPER" rustc
    ;;
  focused)
    echo "focused $TMPDIR" >>"$SELFTEST_SEEN"
    : >"$TMPDIR/left-by-a-test"
    exit 7
    ;;
  "nextest run --workspace")
    echo "test $TMPDIR" >>"$SELFTEST_SEEN"
    : >"$TMPDIR/left-by-a-test"
    "$RUSTC_WRAPPER" rustc
    echo "Summary [1s] 1 tests run: 1 passed, 0 skipped"
    exit "$SELFTEST_SUITE_STATUS"
    ;;
  *) echo "test result: ok. 1 passed; 0 failed; 0 ignored" ;;
esac
EOF
  cat >"$fixture/bin/sccache" <<'EOF'
#!/usr/bin/env bash
echo "$TMPDIR" >>"$SELFTEST_SERVER_TMPDIR"
EOF
  chmod +x "$fixture/bin/cargo" "$fixture/bin/sccache"
  owned_run() { # owned_run <name> <caller> <suite status> <gate args...>; sets rc
    local name="$1" caller="$2" suite_status="$3"
    shift 3
    mkdir -p "$caller"
    (
      cd "$fixture/repo" || exit 3
      [ -z "$explicit_wrapper" ] || export RUSTC_WRAPPER="$explicit_wrapper"
      PATH="$fixture/bin:$PATH" TMPDIR="$caller" \
        SELFTEST_CARGO_VERSION="$real_cargo_version" \
        SELFTEST_SEEN="$fixture/seen-$name" \
        SELFTEST_SUITE_STATUS="$suite_status" \
        SELFTEST_SERVER_TMPDIR="$fixture/server-$name" \
        CARGO_CI_NO_LOCK=1 \
        "$gate" "$@"
    ) >"$fixture/out-$name" 2>&1
    rc=$?
  }
  local rc caller explicit_wrapper=""
  caller="$fixture/caller-explicit"
  cp "$fixture/bin/sccache" "$fixture/bin/explicit-wrapper"
  explicit_wrapper="$fixture/bin/explicit-wrapper"
  owned_run explicit "$caller" 0 --suite
  explicit_wrapper=""
  check "explicit RUSTC_WRAPPER: the caller's wrapper starts with the caller's TMPDIR, in the probe and a compile" \
    test "$(sort -u "$fixture/server-explicit")" = "$caller" -a "$(wc -l <"$fixture/server-explicit")" -ge 2
  for status in 0 1; do
    caller="$fixture/caller-$status"
    owned_run "suite-$status" "$caller" "$status" --suite
    check "run owning its TMPDIR (suite status $status): the wrapper probe sees a directory inside the caller's" \
      grep -qF "probe $caller/" "$fixture/seen-suite-$status"
    check "run owning its TMPDIR (suite status $status): the tests see a directory inside the caller's" \
      grep -qF "test $caller/" "$fixture/seen-suite-$status"
    check "run owning its TMPDIR (suite status $status): a compiler-cache server started by the probe or a compile keeps the caller's" \
      test "$(sort -u "$fixture/server-suite-$status")" = "$caller" -a "$(wc -l <"$fixture/server-suite-$status")" -ge 2
    check "run owning its TMPDIR (suite status $status): a file a test left is gone and nothing else remains" \
      test -z "$(ls -A "$caller")"
  done

  caller="$fixture/caller-focused"
  owned_run focused "$caller" 0 --cargo focused
  check "focused --cargo: Cargo sees a directory inside the caller's" \
    grep -qF "focused $caller/" "$fixture/seen-focused"
  check "focused --cargo: the run exits with Cargo's status" test "$rc" -eq 7
  check "focused --cargo: nothing remains in the caller's TMPDIR" test -z "$(ls -A "$caller")"
}

# --- fixture construction ----------------------------------------------------

# Base commit of a throwaway crate: dependency-free, two independent modules.
# A main-side change touches one and a worker-side change touches the other, so
# every merge below is textually clean by construction. src/claims_log.rs is
# present but undeclared — cargo ignores a .rs file no `mod` names, so the base
# builds — and the resurrection scenario deletes it on one side while declaring
# it on the other.
seed_base() {
  local repo="$1"
  mkdir -p "$repo/src"
  cat >"$repo/Cargo.toml" <<'EOF'
[workspace]

[package]
name = "merge-gate-fixture"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"
EOF
  cat >"$repo/src/lib.rs" <<'EOF'
pub mod alpha;
pub mod omega;
EOF
  cat >"$repo/src/alpha.rs" <<'EOF'
pub fn value() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::value;

    #[test]
    fn test_value_returns_the_base_value() {
        assert_eq!(value(), 1);
    }
}
EOF
  cat >"$repo/src/omega.rs" <<'EOF'
pub fn label() -> &'static str {
    "omega"
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn test_label_names_the_module() {
        assert_eq!(label(), "omega");
    }
}
EOF
  printf 'pub fn record() {}\n' >"$repo/src/claims_log.rs"

  git -C "$repo" init -q
  git -C "$repo" config user.email selftest@jit
  git -C "$repo" config user.name selftest
  git -C "$repo" add -A
  git -C "$repo" commit -qm "base: two independent modules, undeclared claims_log.rs"
}

# Seeds the base, applies the main-side change to the mainline and the
# worker-side change to a branch anchored at the base, then merges. Returns
# nonzero when git reports a conflict — every scenario here must merge cleanly,
# which is the point.
build_merge() { # build_merge <repo> <main-side-fn> <worker-side-fn>
  local repo="$1" main_side="$2" worker_side="$3" mainline
  seed_base "$repo" || return 1
  mainline=$(git -C "$repo" rev-parse --abbrev-ref HEAD)
  git -C "$repo" branch worker

  "$main_side" "$repo"
  git -C "$repo" add -A
  git -C "$repo" commit -qm "mainline change"

  git -C "$repo" checkout -q worker
  "$worker_side" "$repo"
  git -C "$repo" add -A
  git -C "$repo" commit -qm "worker change"

  git -C "$repo" checkout -q "$mainline"
  git -C "$repo" merge --no-ff -q worker -m "merge worker into the mainline"
}

# Healthy control: both sides extend their own module. Each side rewrites its
# whole file rather than appending, so no fixture ends up with an item after its
# test module — a lint failure would be noise no scenario here is about.
healthy_mainline() {
  cat >"$1/src/alpha.rs" <<'EOF'
pub fn value() -> u32 {
    1
}

pub fn doubled() -> u32 {
    value() * 2
}

#[cfg(test)]
mod tests {
    use super::{doubled, value};

    #[test]
    fn test_value_returns_the_base_value() {
        assert_eq!(value(), 1);
    }

    #[test]
    fn test_doubled_returns_twice_the_value() {
        assert_eq!(doubled(), 2);
    }
}
EOF
}

healthy_worker() {
  cat >"$1/src/omega.rs" <<'EOF'
pub fn label() -> &'static str {
    "omega"
}

pub fn shouted() -> String {
    label().to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::{label, shouted};

    #[test]
    fn test_label_names_the_module() {
        assert_eq!(label(), "omega");
    }

    #[test]
    fn test_shouted_upcases_the_label() {
        assert_eq!(shouted(), "OMEGA");
    }
}
EOF
}

ignored_test_worker() {
  cat >"$1/src/omega.rs" <<'EOF'
pub fn label() -> &'static str {
    "omega"
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn test_label_names_the_module() {
        assert_eq!(label(), "omega");
    }

    #[test]
    #[ignore]
    fn test_held_out_by_ignore_attribute() {
        assert_eq!(label(), "held out");
    }
}
EOF
}

ignored_doctest_worker() {
  cat >"$1/src/omega.rs" <<'EOF'
pub fn label() -> &'static str {
    "omega"
}

/// A doctest held out of the default run.
///
/// ```ignore
/// assert_eq!(ignored_doctest_is_named(), 42);
/// ```
pub fn ignored_doctest_is_named() -> u32 {
    42
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn test_label_names_the_module() {
        assert_eq!(label(), "omega");
    }
}
EOF
}

# Resurrection: the mainline deletes the dead file, the worker starts declaring
# it. The merge commit names a module whose file is gone (error[E0583]).
resurrection_mainline() { git -C "$1" rm -q src/claims_log.rs; }

resurrection_worker() { printf 'pub mod claims_log;\n' >>"$1/src/lib.rs"; }

# Signature drift: the mainline gives `value` a parameter and updates its own
# caller; the worker adds a test-only caller of the old form. Only test code
# breaks, so `cargo build` never sees it.
signature_mainline() {
  cat >"$1/src/alpha.rs" <<'EOF'
pub fn value(scale: u32) -> u32 {
    scale
}

#[cfg(test)]
mod tests {
    use super::value;

    #[test]
    fn test_value_returns_its_scale() {
        assert_eq!(value(3), 3);
    }
}
EOF
}

signature_worker() {
  cat >"$1/src/omega.rs" <<'EOF'
pub fn label() -> &'static str {
    "omega"
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn test_label_names_the_module() {
        assert_eq!(label(), "omega");
    }

    #[test]
    fn test_alpha_value_is_the_base_value() {
        assert_eq!(crate::alpha::value(), 1);
    }
}
EOF
}

# Stale expectation: the mainline changes what `value` returns and updates its
# own test; the worker adds a test asserting the pre-merge value. The merge
# compiles completely and fails when the tests run.
stale_expect_mainline() {
  cat >"$1/src/alpha.rs" <<'EOF'
pub fn value() -> u32 {
    2
}

#[cfg(test)]
mod tests {
    use super::value;

    #[test]
    fn test_value_returns_the_revised_value() {
        assert_eq!(value(), 2);
    }
}
EOF
}

stale_expect_worker() { signature_worker "$1"; }

# --- running the shipped gate ------------------------------------------------

# Runs scripts/cargo-ci.sh — the real one, at its real path — over the merged
# tree, capturing its combined output. CARGO_TARGET_DIR is cleared so the
# fixture builds into its own throwaway target rather than into a target
# directory this run inherited, and the host-wide build lock is skipped: this
# self-test itself runs inside a gate run that already holds that lock, and the
# fixture is far too small to need it. These fixtures compile for real, so they
# opt out of sccache: the host daemon's health is no part of their verdict.
run_gate_script() { # run_gate_script <script> <repo> <output-file> [VAR=value ...]
  local script="$1" repo="$2" out="$3"
  shift 3
  (
    cd "$repo" || exit 3
    unset CARGO_TARGET_DIR
    env CARGO_CI_NO_LOCK=1 CARGO_CI_NO_SCCACHE=1 "$@" "$script"
  ) >"$out" 2>&1
}

run_gate() { # run_gate <repo> <output-file> [VAR=value ...]
  local repo="$1" out="$2"
  shift 2
  run_gate_script "$gate" "$repo" "$out" "$@"
}

# The removed merge guard's command, run over the same tree.
build_only_passes() { # build_only_passes <repo>
  (
    cd "$1" || exit 3
    unset CARGO_TARGET_DIR
    CARGO_INCREMENTAL=0 cargo build --workspace
  ) >/dev/null 2>&1
}

run_scenario() { # run_scenario <name> <main-fn> <worker-fn> <pass|fail> [build-only-passes] [runtime-reporter]
  local name="$1" main_side="$2" worker_side="$3" expect="$4" build_only="${5:-}" reporter="${6:-}"
  local repo="$scratch/$name" out="$scratch/$name.gate.out" rc

  echo
  echo "== $name: the gate's build-and-test step must $expect =="

  if ! build_merge "$repo" "$main_side" "$worker_side"; then
    echo "FAIL: $name: expected a textually clean merge, git reported a conflict"
    fail=1
    return
  fi
  echo "PASS: $name: the branches merge without a conflict"
  check "$name: the merged working tree is the merge commit's tree" \
    test -z "$(git -C "$repo" status --porcelain)"

  run_gate "$repo" "$out"
  rc=$?

  case "$expect" in
    pass)
      check "$name: the gate reports its build-and-test step passing" \
        step_passed "$out" test
      check "$name: every stable step summary records integer milliseconds" \
        all_step_summaries_have_integer_milliseconds "$out"
      check "$name: the nextest success reporter preserves all four tests" \
        nextest_success_reported "$out"
      check "$name: the gate reports its separate doctest step passing" \
        step_passed "$out" doctest
      check "$name: the gate reports its excluded-tests step passing" \
        step_passed "$out" excluded-tests
      ;;
    fail)
      check "$name: the gate reports its build-and-test step failing" \
        step_failed "$out" test
      check "$name: failed step summaries retain integer milliseconds" \
        all_step_summaries_have_integer_milliseconds "$out"
      check "$name: the gate exits nonzero" test "$rc" -ne 0
      ;;
  esac

  if [ "$build_only" = "build-only-passes" ]; then
    check "$name: a build-only check passes on this same tree" \
      build_only_passes "$repo"
  fi

  if [ "$reporter" = "runtime-reporter" ]; then
    check "$name: the nextest failure reporter preserves the failing test and totals" \
      nextest_runtime_failure_reported "$out"
  fi
}

test_ignored_test_is_rejected() {
  local name="ignored-test" repo="$scratch/ignored-test" out="$scratch/ignored-test.gate.out" rc

  echo
  echo "== $name: an ignored test must fail the excluded-tests step =="
  if ! build_merge "$repo" "healthy_mainline" "ignored_test_worker"; then
    echo "FAIL: $name: expected a textually clean merge, git reported a conflict"
    fail=1
    return
  fi
  run_gate "$repo" "$out"
  rc=$?

  check "$name: the test step still passes" step_passed "$out" test
  check "$name: the excluded-tests step fails" step_failed "$out" excluded-tests
  check "$name: the gate exits nonzero" test "$rc" -ne 0
  check "$name: the diagnostic names the held-out test" \
    grep -qF "test_held_out_by_ignore_attribute" "$out"
}

test_ignored_doctest_is_rejected() {
  local name="ignored-doctest" repo="$scratch/ignored-doctest" out="$scratch/ignored-doctest.gate.out" rc

  echo
  echo "== $name: an ignored doctest must fail the excluded-tests step =="
  if ! build_merge "$repo" "healthy_mainline" "ignored_doctest_worker"; then
    echo "FAIL: $name: expected a textually clean merge, git reported a conflict"
    fail=1
    return
  fi
  run_gate "$repo" "$out"
  rc=$?

  check "$name: the test step still passes" step_passed "$out" test
  check "$name: the excluded-tests step fails" step_failed "$out" excluded-tests
  check "$name: the gate exits nonzero" test "$rc" -ne 0
  check "$name: the diagnostic names the held-out doctest" \
    grep -qF "ignored_doctest_is_named" "$out"
}

# --- the per-test limits -----------------------------------------------------
# The shipped .config/nextest.toml, copied verbatim into a fixture holding one
# unit and one integration test that never finish, must terminate each at its
# own layer's limit and fail the run naming it. The limits are read from the
# shipped configuration, so the assertions follow it rather than copy it.
over_limit_worker() {
  local repo="$1"
  mkdir -p "$repo/.config" "$repo/tests"
  cp "$here/../.config/nextest.toml" "$repo/.config/nextest.toml"
  cat >"$repo/src/omega.rs" <<'EOF'
pub fn label() -> &'static str {
    "omega"
}

pub fn wait_forever() {
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_unit_layer_test_never_finishes() {
        super::wait_forever();
    }
}
EOF
  cat >"$repo/tests/over_limit.rs" <<'EOF'
#[test]
fn test_integration_layer_test_never_finishes() {
    merge_gate_fixture::omega::wait_forever();
}
EOF
}

# Seconds nextest reported on the TIMEOUT line naming test <name>.
timeout_seconds() { # timeout_seconds <output-file> <name>
  grep -E "^[[:space:]]*TIMEOUT \[" "$1" | grep -F "$2" | head -1 |
    sed -E 's/^[[:space:]]*TIMEOUT \[[[:space:]]*([0-9.]+)s\].*/\1/'
}

# The shipped limit in seconds for the unit layer (profile default) or the
# integration layer (the `kind(test)` override).
shipped_limit_seconds() { # shipped_limit_seconds unit|integration
  python3 -I - "$here/../.config/nextest.toml" "$1" <<'EOF'
import sys, tomllib
profile = tomllib.load(open(sys.argv[1], "rb"))["profile"]["default"]
timeout = profile["slow-timeout"]
if sys.argv[2] == "integration":
    timeout = next(o["slow-timeout"] for o in profile["overrides"] if o["filter"] == "kind(test)")
period = timeout["period"]
assert period.endswith("s") and period[:-1].isdigit(), period
print(int(period[:-1]) * timeout["terminate-after"])
EOF
}

# lower <= value < upper, for decimal seconds.
seconds_within() { # seconds_within <value> <lower> <upper>
  [ -n "$1" ] && awk -v v="$1" -v lo="$2" -v hi="$3" 'BEGIN { exit !(v >= lo && v < hi) }'
}

test_test_over_its_layer_limit_fails_the_run() {
  local name="over-limit" repo="$scratch/over-limit" out="$scratch/over-limit.gate.out" rc
  local unit_limit integration_limit unit_s integration_s

  echo
  echo "== $name: a test over its layer's per-test limit must fail the test step =="
  seed_base "$repo" || return 1
  over_limit_worker "$repo"
  unit_limit=$(shipped_limit_seconds unit) || return 1
  integration_limit=$(shipped_limit_seconds integration) || return 1
  run_gate "$repo" "$out"
  rc=$?
  unit_s=$(timeout_seconds "$out" test_unit_layer_test_never_finishes)
  integration_s=$(timeout_seconds "$out" test_integration_layer_test_never_finishes)

  check "$name: the test step fails" step_failed "$out" test
  check "$name: the gate exits nonzero" test "$rc" -ne 0
  check "$name: the unit test is named TIMEOUT at the unit limit, before the integration limit" \
    seconds_within "$unit_s" "$unit_limit" "$integration_limit"
  check "$name: the integration test is named TIMEOUT at the integration limit" \
    seconds_within "$integration_s" "$integration_limit" "$((integration_limit * 2))"
}

# --- the exit-status classifier ----------------------------------------------
# A failure is an infrastructure fault (75) only when every failing step ran out
# of disk space (ENOSPC, whatever the load), or is a test TIMEOUT with no FAIL
# on a host that was loaded before the run started; everything else is a
# verdict (1). Synthetic nextest and
# budget output stand in for the steps, so no suite runs.
readonly LOADED_LOAD1=9.00 QUIET_LOAD1=0.50 FIXTURE_CPUS=4
# Baselines on each side of the 0.17 x FIXTURE_CPUS = 0.68 threshold.
readonly JUST_BELOW_LOAD1=0.67 AT_THRESHOLD_LOAD1=0.68
readonly TIMEOUT_LINE='        TIMEOUT [  60.001s] jit commands::slow::test_slow'
readonly FAIL_LINE='        FAIL [   0.120s] jit commands::x::test_broken'
readonly ENOSPC_FAIL_BLOCK='        FAIL [   0.200s] jit profile_cli_tests::test_pack
--- STDERR: jit profile_cli_tests::test_pack ---
thread test_pack panicked at crates/jit/tests/pack.rs:12:5:
create the bounded package asset directory: Os { code: 28, kind: StorageFull, message: "No space left on device" }
note: run with RUST_BACKTRACE=1 environment variable to display a backtrace'
readonly ENOSPC_ASSERT_BLOCK='        FAIL [   0.200s] jit profile_cli_tests::test_add
--- STDERR: jit profile_cli_tests::test_add ---
thread test_add panicked at crates/jit/tests/add.rs:30:5:
command failed: Output { status: ExitStatus(1), stdout: "", stderr: "write: No space left on device (os error 28)" }
note: run with RUST_BACKTRACE=1 environment variable to display a backtrace'
readonly MIXED_BLOCK='        FAIL [   0.200s] jit gate_tests::test_digest
--- STDOUT: jit gate_tests::test_digest ---
log: write failed: No space left on device (os error 28)
--- STDERR: jit gate_tests::test_digest ---
thread test_digest panicked at crates/jit/tests/digest.rs:9:5:
assertion left == right failed
  left: Stable
 right: InputsMoved
note: run with RUST_BACKTRACE=1 environment variable to display a backtrace
log: later No space left on device'
readonly FOOTPRINT_VIOLATION='  - integration-test targets: observed 99, limit 10.'

classify() { # classify <load1> <test-output> <budget-output> <failed-step...>
  local load="$1" dir="$scratch/classify"
  shift
  rm -rf "$dir"
  mkdir -p "$dir"
  printf '%s\n' "$1" >"$dir/test.out"
  printf '%s\n' "$2" >"$dir/budget.out"
  shift 2
  "$gate" --classify "$dir" "$load" "$FIXTURE_CPUS" "$@" >"$dir/diagnosis" 2>&1
  classify_rc=$?
}

expect_classification() { # expect_classification <message> <exit> <load1> <test> <budget> <steps...>
  local msg="$1" want="$2"
  shift 2
  classify "$@"
  check "$msg: exits $want" test "$classify_rc" -eq "$want"
  grep -q 'failed on repository content' "$scratch/classify/diagnosis" ||
    check "$msg: the diagnosis records the observed load" \
      grep -q "baseline load1=${1:-unknown}" "$scratch/classify/diagnosis"
  [ "$want" -ne 75 ] ||
    check "$msg: the diagnosis records the classifying threshold" \
      grep -qF "threshold 0.68 (0.17 x ${FIXTURE_CPUS} CPUs)" "$scratch/classify/diagnosis"
}

test_classifier_exit_paths() {
  echo
  echo "== classifier: only a loaded-host timeout exits 75 =="
  expect_classification "loaded timeout without FAIL" 75 "$LOADED_LOAD1" "$TIMEOUT_LINE" "" test
  expect_classification "quiet timeout" 1 "$QUIET_LOAD1" "$TIMEOUT_LINE" "" test
  expect_classification "loaded timeout with a failing test" 1 "$LOADED_LOAD1" "$TIMEOUT_LINE"$'\n'"$FAIL_LINE" "" test
  expect_classification "loaded fmt failure" 1 "$LOADED_LOAD1" "" "" fmt
  expect_classification "loaded clippy failure" 1 "$LOADED_LOAD1" "" "" clippy
  expect_classification "loaded doctest failure" 1 "$LOADED_LOAD1" "" "" doctest
  expect_classification "loaded footprint budget failure" 1 "$LOADED_LOAD1" "" "$FOOTPRINT_VIOLATION" budget
  expect_classification "loaded timeout beside a clippy failure" 1 "$LOADED_LOAD1" "$TIMEOUT_LINE" "" test clippy
  expect_classification "loaded test step failing without any TIMEOUT result" 1 "$LOADED_LOAD1" "error: could not compile" "" test
  expect_classification "timeout with a baseline just below the threshold" 1 "$JUST_BELOW_LOAD1" "$TIMEOUT_LINE" "" test
  expect_classification "timeout with a baseline at the threshold" 75 "$AT_THRESHOLD_LOAD1" "$TIMEOUT_LINE" "" test
  expect_classification "unknown load" 1 "" "$TIMEOUT_LINE" "" test
}

expect_enospc_classification() { # expect_enospc_classification <message> <exit> <load1> <test> <budget> <steps...>
  local msg="$1" want="$2"
  shift 2
  classify "$@"
  check "$msg: exits $want" test "$classify_rc" -eq "$want"
  [ "$want" -ne 75 ] ||
    check "$msg: the diagnosis names ENOSPC as the cause" grep -qF 'ENOSPC' "$scratch/classify/diagnosis"
}

test_classifier_enospc_exit_paths() {
  echo
  echo "== classifier: ENOSPC exits 75 whatever the load, unless a genuine failure joins it =="
  local beside_fail="$ENOSPC_FAIL_BLOCK"$'\n'"$FAIL_LINE"$'\n'"thread 'x' panicked at a.rs:1:1:"$'\n'"assertion failed"
  expect_enospc_classification "ENOSPC test failure on a quiet host" 75 "$QUIET_LOAD1" "$ENOSPC_FAIL_BLOCK" "" test
  expect_enospc_classification "ENOSPC test failure on a loaded host" 75 "$LOADED_LOAD1" "$ENOSPC_FAIL_BLOCK" "" test
  expect_enospc_classification "ENOSPC test failure with unknown load" 75 "" "$ENOSPC_FAIL_BLOCK" "" test
  expect_enospc_classification "ENOSPC budget failure on a quiet host" 75 "$QUIET_LOAD1" "" "write failed: No space left on device" budget
  expect_enospc_classification "ENOSPC clippy-style error alone" 75 "$QUIET_LOAD1" "" "error: failed to write: No space left on device (os error 28)
error: could not compile \`x\` due to 1 previous error" budget
  expect_enospc_classification "ENOSPC beside a compiler error in a non-test step" 1 "$QUIET_LOAD1" "" "error: failed to write: No space left on device (os error 28)
error[E0308]: mismatched types" budget
  expect_enospc_classification "ENOSPC beside a footprint violation in a non-test step" 1 "$QUIET_LOAD1" "" "write failed: No space left on device
$FOOTPRINT_VIOLATION" budget
  expect_enospc_classification "ENOSPC assert-message failure" 75 "$QUIET_LOAD1" "$ENOSPC_ASSERT_BLOCK" "" test
  expect_enospc_classification "ENOSPC elsewhere in a block whose panic is an assertion" 1 "$QUIET_LOAD1" "$MIXED_BLOCK" "" test
  expect_enospc_classification "one ENOSPC panic and one assertion panic in a block" 1 "$QUIET_LOAD1" "$ENOSPC_FAIL_BLOCK"$'\n'"thread 'y' panicked at b.rs:2:2:"$'\n'"assertion failed" "" test
  expect_enospc_classification "ENOSPC failure beside a genuine test FAIL" 1 "$LOADED_LOAD1" "$beside_fail" "" test
  expect_enospc_classification "ENOSPC failure beside a genuine FAIL on a quiet host" 1 "$QUIET_LOAD1" "$beside_fail" "" test
  expect_enospc_classification "ENOSPC test step beside a footprint budget failure" 1 "$QUIET_LOAD1" "$ENOSPC_FAIL_BLOCK" "$FOOTPRINT_VIOLATION" test budget
}

# Without errexit a test that cannot run, such as a missing function, would only
# print to stderr. Each test ends with a status-0 assertion, so nonzero means it did
# not complete.
run_test() { # run_test <function> [args...]
  "$@" || {
    echo "FAIL: $1 did not complete (status $?)"
    fail=1
  }
}

echo
echo "== wrong-nextest-version: the gate must fail before its first step =="
run_test test_wrong_nextest_version_fails_fast
run_test test_suite_mode_runs_only_the_suite_steps
run_test test_fail_fast_timeout_fails_only_the_test_step
run_test test_classifier_exit_paths
run_test test_classifier_enospc_exit_paths
run_test test_build_lock_status_propagation
run_test test_script_build_lock_wait
run_test test_unrunnable_compiler_wrapper
run_test test_run_removes_its_temporary_directory

run_test run_scenario healthy "healthy_mainline" "healthy_worker" pass
run_test run_scenario resurrection "resurrection_mainline" "resurrection_worker" fail
run_test run_scenario signature "signature_mainline" "signature_worker" fail build-only-passes
run_test run_scenario stale-expect "stale_expect_mainline" "stale_expect_worker" fail build-only-passes runtime-reporter
run_test test_ignored_test_is_rejected
run_test test_ignored_doctest_is_rejected
run_test test_test_over_its_layer_limit_fails_the_run

echo
if [ "$fail" -eq 0 ]; then
  echo "SELFTEST: all assertions passed"
else
  echo "SELFTEST: assertions FAILED"
fi
exit "$fail"
