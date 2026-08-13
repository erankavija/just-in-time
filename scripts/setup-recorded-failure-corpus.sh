#!/usr/bin/env bash
set -euo pipefail

readonly PINNED_NEXTEST_VERSION="0.9.133"
readonly SETUP_TEST="failure_probe_fixture::test_failure_probe_fixture_accepts_every_recorded_setup_step"

fail_environment() {
  echo "recorded-failure corpus setup: $1" >&2
  exit 2
}

run_setup() {
  [ "${NEXTEST:-}" = "1" ] || fail_environment "must be invoked by cargo-nextest"
  [ "${NEXTEST_VERSION:-}" = "$PINNED_NEXTEST_VERSION" ] ||
    fail_environment "requires cargo-nextest $PINNED_NEXTEST_VERSION; observed ${NEXTEST_VERSION:-unset}"
  [ -n "${NEXTEST_ENV:-}" ] || fail_environment "NEXTEST_ENV is not set"
  [ -n "${NEXTEST_WORKSPACE_ROOT:-}" ] || fail_environment "NEXTEST_WORKSPACE_ROOT is not set"

  # Package selection is `--workspace` to match the build the caller already
  # paid for, not because this setup needs the server crate (jit:0708d692).
  # Cargo unifies features over the packages one invocation selects, so
  # `-p jit` resolves a second variant of the whole dependency graph and
  # compiles it: measured on a target freshly built by `cargo test --workspace
  # --no-run`, `-p jit --test cli_issue` compiled 60 crates in 24,677 ms while
  # `--workspace --test cli_issue` was fresh in 131 ms. cargo-nextest runs this
  # script inside the suite run, so that duplicate build landed inside the
  # measured suite clock on every fresh target — the clock the `budget` step
  # judges — and its artifacts inside the enforced build footprint.
  (
    cd "$NEXTEST_WORKSPACE_ROOT"
    unset JIT_RECORDED_FAILURE_CORPUS_RECEIPT
    unset JIT_RECORDED_FAILURE_CORPUS_RECEIPT_SHA256
    CARGO_INCREMENTAL=0 JIT_RECORDED_FAILURE_CORPUS_SETUP=1 \
      "${CARGO:-cargo}" test --workspace --test cli_issue "$SETUP_TEST" -- --exact --nocapture
  )

  grep -q '^JIT_RECORDED_FAILURE_CORPUS_RECEIPT=' "$NEXTEST_ENV" ||
    fail_environment "setup test did not publish the receipt"
  grep -q '^JIT_RECORDED_FAILURE_CORPUS_RECEIPT_SHA256=' "$NEXTEST_ENV" ||
    fail_environment "setup test did not publish the receipt SHA"
}

self_test() {
  local scratch fake_cargo log env_file mismatch_log rc
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' RETURN
  fake_cargo="$scratch/cargo"
  log="$scratch/cargo.log"
  env_file="$scratch/nextest.env"
  mismatch_log="$scratch/mismatch.log"
  cat >"$fake_cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$SETUP_SELFTEST_LOG"
printf '%s\n' \
  'JIT_RECORDED_FAILURE_CORPUS_RECEIPT=/verified/receipt.json' \
  'JIT_RECORDED_FAILURE_CORPUS_RECEIPT_SHA256=0123456789abcdef' >>"$NEXTEST_ENV"
EOF
  chmod +x "$fake_cargo"

  NEXTEST=1 NEXTEST_VERSION="$PINNED_NEXTEST_VERSION" \
    NEXTEST_WORKSPACE_ROOT="$PWD" NEXTEST_ENV="$env_file" \
    CARGO="$fake_cargo" SETUP_SELFTEST_LOG="$log" run_setup
  grep -qF "test --workspace --test cli_issue $SETUP_TEST -- --exact --nocapture" "$log"

  set +e
  (
    NEXTEST=1 NEXTEST_VERSION="0.9.132" NEXTEST_WORKSPACE_ROOT="$PWD" \
      NEXTEST_ENV="$scratch/mismatch.env" CARGO="$fake_cargo" \
      SETUP_SELFTEST_LOG="$mismatch_log" run_setup
  ) >/dev/null 2>&1
  rc=$?
  set -e
  [ "$rc" -eq 2 ] || fail_environment "version mismatch self-test expected exit 2, got $rc"
  [ ! -e "$mismatch_log" ] || fail_environment "version mismatch reached Cargo"
  echo "recorded-failure corpus setup self-test: PASS"
}

case "${1:-}" in
  --self-test) self_test ;;
  "") run_setup ;;
  *) fail_environment "unknown argument: $1" ;;
esac
