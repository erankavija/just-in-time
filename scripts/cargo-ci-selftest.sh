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
#
# The signature and stale-expect scenarios also assert that `cargo build
# --workspace` SUCCEEDS on the same tree: that is the recorded reason a
# build-only merge guard was vacuous, kept here as a regression so the
# distinction cannot quietly be lost again.
#
# Three further scenarios (jit:0708d692) assert how the gate judges its
# incremental-compilation ban, since a verdict about the machine is not a
# verdict about the tree. What decides that verdict is the ban's runtime value,
# not the contents of a shared directory no shell can attribute to a writer:
#
#   incremental-pre-existing incremental state another process wrote before the
#                            run              -> recorded, ignored, gate passes
#   incremental-concurrent   incremental state that appears while the run works,
#                            as an editor indexing the same checkout writes it
#                                              -> reported, gate passes
#   incremental-ban-lifted   a gate whose CARGO_INCREMENTAL=0 export is gone,
#                            run with the ban lifted in its environment
#                                              -> the incremental verdict fails
#
# Cost: every fixture is a dependency-free two-module crate in a throwaway git
# repo with its own target directory. This workspace is never rebuilt.
#
# Exit codes:
#   0 — all assertions passed
#   1 — one or more assertions failed
#   2 — environment error (missing tooling)

for tool in git cargo jq; do
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

fail=0
real_cargo=$(command -v cargo)

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
  summaries=$(grep -E '^  [✓✗] [a-z-]+: ' "$1" | awk '$2 != "suite-clock:"')
  [ -n "$summaries" ] &&
    ! grep -Ev '^  [✓✗] [a-z-]+: .+ \([0-9]+ ms\)$' <<<"$summaries"
}

suite_clock_is_reported() {
  grep -Eq '^  ✓ suite-clock: [0-9]+ ms$' "$1"
}

# jit:0708d692: the step that prepares the measured suite keeps the costs of
# getting there out of the clock — compiling the suite, and reading the
# executables it linked back into the page cache — and reports both. A run that
# warmed nothing is the regression this predicate catches; the counts come from
# the gate's own report of what it did, not from a fixed expectation about a
# fixture's size.
suite_preparation_warms_the_linked_executables() {
  local report warmed bytes
  report=$(grep -E '^  ✓ suite-build: ' "$1" | tail -1)
  warmed=$(grep -oP 'warmed \K[0-9]+(?= executables)' <<<"$report")
  bytes=$(grep -oP 'warmed [0-9]+ executables \(\K[0-9]+' <<<"$report")
  [ -n "$warmed" ] && [ -n "$bytes" ] && [ "$warmed" -gt 0 ] && [ "$bytes" -gt 0 ]
}

# Those costs must be paid before the clock starts, which the reported order of
# the summary lines records: the summary is accumulated step by step.
suite_preparation_precedes_the_clock() {
  awk '
    /^  ✓ suite-build: built in [0-9]+ ms.*warmed [0-9]+ executables/ { prepared = NR }
    /^  ✓ suite-clock: [0-9]+ ms$/ { clock = NR }
    END { exit !(prepared && clock && prepared < clock) }
  ' "$1"
}

# The scope is a source-order contract rather than an elapsed-time assertion:
# the timer must bracket the two independently reported suite substeps and
# close immediately after them. This remains deterministic on loaded hosts.
suite_clock_has_exact_substep_scope() {
  awk '
    /suite_clock_started_ms=\$\(epoch_milliseconds\)/ { start = NR }
    /run_step test .*cargo nextest run --workspace/ { nextest = NR }
    /run_step doctest .*cargo test --doc --workspace/ { doctest = NR }
    /suite_clock_ms=\$\(\(.*suite_clock_started_ms/ { stop = NR }
    END { exit !(start < nextest && nextest < doctest && doctest < stop) }
  ' "$gate"
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
# before it reaches fmt or another gate step. A cargo shim delegates the real
# cargo probe but injects an older nextest version; any later Cargo invocation
# is an assertion failure in the fixture itself.
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
  exec "$SELFTEST_REAL_CARGO" "$@"
fi
if [ "${1:-}" = "nextest" ] && [ "${2:-}" = "--version" ]; then
  echo "cargo-nextest 0.9.132 (injected selftest fixture)"
  exit 0
fi
printf '%s\n' "$*" >"$SELFTEST_UNEXPECTED_CARGO"
exit 97
EOF
  chmod +x "$fake_bin/cargo"

  (
    cd "$fixture/repo" || exit 3
    PATH="$fake_bin:$PATH" \
      SELFTEST_REAL_CARGO="$real_cargo" \
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
# fixture is far too small to need it.
run_gate_script() { # run_gate_script <script> <repo> <output-file> [VAR=value ...]
  local script="$1" repo="$2" out="$3"
  shift 3
  (
    cd "$repo" || exit 3
    unset CARGO_TARGET_DIR
    env CARGO_CI_NO_LOCK=1 "$@" "$script"
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
      check "$name: the gate reports the named suite clock" \
        suite_clock_is_reported "$out"
      check "$name: the suite clock brackets only nextest and doctests" \
        suite_clock_has_exact_substep_scope
      check "$name: the suite preparation reads the executables it linked into the page cache" \
        suite_preparation_warms_the_linked_executables "$out"
      check "$name: the suite is compiled and warmed before the clock starts" \
        suite_preparation_precedes_the_clock "$out"
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

# --- how the gate judges its incremental ban (jit:0708d692) ------------------

# The incremental state another process leaves in a target directory it shares
# with the gate: one per-crate directory holding one session directory, the
# shape rustc writes. An editor's rust-analyzer writes exactly this, in every
# checkout it has open, and repopulates it within seconds of it being cleared.
seed_foreign_incremental_state() { # seed_foreign_incremental_state <repo>
  local session="$1/target/debug/incremental/foreign-crate-1a2b3c/s-foreign-session-working"
  mkdir -p "$session"
  printf 'written by another process before the gate ran\n' >"$session/dep-graph.bin"
}

foreign_incremental_state_survives() { # foreign_incremental_state_survives <repo>
  test -f "$1/target/debug/incremental/foreign-crate-1a2b3c/s-foreign-session-working/dep-graph.bin"
}

# A healthy merge whose target directory already holds another process's
# incremental state. The gate must judge the tree, so it records that state and
# proceeds; before jit:0708d692 it refused at a preflight step and never reached
# the tests, which is the failure mode these assertions pin.
test_pre_existing_incremental_state_does_not_fail_the_gate() {
  local name="incremental-pre-existing"
  local repo="$scratch/$name" out="$scratch/$name.gate.out"

  echo
  echo "== $name: state another process wrote must not decide this gate =="
  if ! build_merge "$repo" "healthy_mainline" "healthy_worker"; then
    echo "FAIL: $name: expected a textually clean merge, git reported a conflict"
    fail=1
    return
  fi
  seed_foreign_incremental_state "$repo"
  run_gate "$repo" "$out"

  check "$name: the gate records the pre-existing state as a baseline instead of refusing to run" \
    step_passed "$out" incremental-baseline
  check "$name: the gate reaches and passes its build-and-test step" \
    step_passed "$out" test
  check "$name: the incremental verdict passes over state this run did not create" \
    step_passed "$out" incremental-state
  check "$name: the verdict says how much pre-existing state it ignored" \
    grep -Eq '✓ incremental-state: .*[0-9]+ pre-existing entries ignored' "$out"
  check "$name: the gate leaves another process's state alone" \
    foreign_incremental_state_survives "$repo"
}

# A worker side whose test writes incremental state into the target directory
# while the gate's own suite runs — after the baseline snapshot, before the
# verdict. That is the ordering an editor's rust-analyzer produces by indexing
# the checkout the gate is working in, reproduced without a sleep or a race: the
# gate itself runs the writer. The directory is named through the environment so
# the fixture holds no assumption about where the gate resolved its target.
concurrent_writer_worker() {
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
    fn test_another_process_writes_incremental_state_while_this_suite_runs() {
        let Ok(session) = std::env::var("SELFTEST_INJECT_INCREMENTAL") else {
            return;
        };
        std::fs::create_dir_all(&session).expect("create the injected session directory");
        std::fs::write(
            std::path::Path::new(&session).join("dep-graph.bin"),
            b"written by another process while the gate was running\n",
        )
        .expect("write the injected session file");
    }
}
EOF
}

# The concurrent case the filesystem comparison could not tell from a regression
# of the gate's own: state that appears mid-run. The ban held for every
# compilation this run performed, so the entries came from elsewhere, and a
# verdict that failed here would be decided by whether an editor happened to
# have the checkout open.
test_concurrent_incremental_state_is_observed_without_failing_the_gate() {
  local name="incremental-concurrent"
  local repo="$scratch/$name" out="$scratch/$name.gate.out"
  local session="target/debug/incremental/concurrent-crate-9f8e7d/s-concurrent-session-working"

  echo
  echo "== $name: state that appears while the run works must not decide it =="
  if ! build_merge "$repo" "healthy_mainline" "concurrent_writer_worker"; then
    echo "FAIL: $name: expected a textually clean merge, git reported a conflict"
    fail=1
    return
  fi
  run_gate "$repo" "$out" "SELFTEST_INJECT_INCREMENTAL=$repo/$session"

  check "$name: the injected state really was written during the run" \
    test -f "$repo/$session/dep-graph.bin"
  check "$name: the gate reaches and passes its build-and-test step" \
    step_passed "$out" test
  check "$name: the incremental verdict passes over state this run did not compile" \
    step_passed "$out" incremental-state
  check "$name: the observation names the entries that appeared" \
    grep -q "concurrent-crate-9f8e7d" "$out"
  check "$name: the observation states the ban that places them outside this run" \
    grep -q 'ban in force (CARGO_INCREMENTAL=0)' "$out"
}

# The regression the policy exists to catch: a gate that compiles without its
# incremental ban in force. The shipped script exports CARGO_INCREMENTAL=0
# itself, and an exported value cannot be overridden from outside it, so the
# only way to observe the assertion firing is to run a gate whose export is
# gone. The mutant is derived from the shipped file at run time by deleting that
# one line — no logic is copied here, and a rename of the export makes this
# fixture fail loudly rather than silently stop testing anything.
gate_without_the_incremental_ban() { # gate_without_the_incremental_ban <destination>
  local dest="$1" export_line='export CARGO_INCREMENTAL=0'
  grep -qxF "$export_line" "$gate" || return 1
  grep -vxF "$export_line" "$gate" >"$dest" || return 1
  chmod +x "$dest"
}

test_lifted_incremental_ban_fails_the_gate() {
  local name="incremental-ban-lifted"
  local repo="$scratch/$name" out="$scratch/$name.gate.out" rc
  local mutant="$scratch/$name-cargo-ci.sh"

  echo
  echo "== $name: a run that compiled without the ban must fail =="
  if ! build_merge "$repo" "healthy_mainline" "healthy_worker"; then
    echo "FAIL: $name: expected a textually clean merge, git reported a conflict"
    fail=1
    return
  fi
  if ! gate_without_the_incremental_ban "$mutant"; then
    echo "FAIL: $name: the shipped gate has no 'export CARGO_INCREMENTAL=0' line to remove"
    fail=1
    return
  fi
  check "$name: the mutant gate no longer exports the ban" \
    test -z "$(grep -xF 'export CARGO_INCREMENTAL=0' "$mutant")"

  run_gate_script "$mutant" "$repo" "$out" CARGO_INCREMENTAL=1
  rc=$?

  check "$name: the incremental verdict fails when the ban was not in force" \
    step_failed "$out" incremental-state
  check "$name: the gate exits nonzero" test "$rc" -ne 0
  check "$name: the diagnostic names the value it observed" \
    grep -q 'observed: CARGO_INCREMENTAL=1' "$out"
  check "$name: the diagnostic names the value it compared against" \
    grep -q 'compared against: the CARGO_INCREMENTAL=0' "$out"
  check "$name: the diagnostic separates the gate's conduct from the machine's state" \
    grep -q "regression in the gate's conduct" "$out"
}

echo
echo "== wrong-nextest-version: the gate must fail before its first step =="
test_wrong_nextest_version_fails_fast

run_scenario healthy "healthy_mainline" "healthy_worker" pass
run_scenario resurrection "resurrection_mainline" "resurrection_worker" fail
run_scenario signature "signature_mainline" "signature_worker" fail build-only-passes
run_scenario stale-expect "stale_expect_mainline" "stale_expect_worker" fail build-only-passes runtime-reporter
test_ignored_test_is_rejected
test_ignored_doctest_is_rejected

test_pre_existing_incremental_state_does_not_fail_the_gate
test_concurrent_incremental_state_is_observed_without_failing_the_gate
test_lifted_incremental_ban_fails_the_gate

echo
if [ "$fail" -eq 0 ]; then
  echo "SELFTEST: all assertions passed"
else
  echo "SELFTEST: assertions FAILED"
fi
exit "$fail"
