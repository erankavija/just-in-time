#!/usr/bin/env bash
set -euo pipefail

# Nextest setup for the six stale-binary semantic tests. The nested build is
# intentionally inside `cargo nextest run`: nextest invokes this helper after
# compiling/listing and before it starts any test selected by the setup rule.
readonly PINNED_NEXTEST_VERSION="0.9.133"
readonly SETUP_TEST="stale_binary_child_process_tests::test_stale_binary_fixture_runs_six_semantic_tests_under_pinned_nextest"

fail_environment() {
  echo "stale-binary fixture setup: $1" >&2
  exit 2
}

run_setup() {
  [ "${NEXTEST:-}" = "1" ] || fail_environment "must be invoked by cargo-nextest"
  [ -n "${NEXTEST_ENV:-}" ] || fail_environment "NEXTEST_ENV is not set"
  [ -n "${NEXTEST_WORKSPACE_ROOT:-}" ] || fail_environment "NEXTEST_WORKSPACE_ROOT is not set"
  [ "${NEXTEST_VERSION:-}" = "$PINNED_NEXTEST_VERSION" ] ||
    fail_environment "requires cargo-nextest $PINNED_NEXTEST_VERSION; observed ${NEXTEST_VERSION:-unset}"

  local cargo_bin="${CARGO:-cargo}"
  (
    cd "$NEXTEST_WORKSPACE_ROOT"
    unset JIT_STALE_FIXTURE_RECEIPT
    unset JIT_STALE_FIXTURE_RECEIPT_SHA256
    unset JIT_STALE_FIXTURE_BUILT_FROM
    JIT_STALE_FIXTURE_SETUP=1 \
      "$cargo_bin" test -p jit --test scratch_build "$SETUP_TEST" -- \
        --exact --nocapture
  )

  for key in \
    JIT_STALE_FIXTURE_RECEIPT \
    JIT_STALE_FIXTURE_RECEIPT_SHA256 \
    JIT_STALE_FIXTURE_BUILT_FROM \
    JIT_STALE_FIXTURE_REUSE_OBSERVATION_DIR; do
    grep -q "^${key}=" "$NEXTEST_ENV" ||
      fail_environment "setup test did not publish $key to NEXTEST_ENV"
  done
}

self_test() {
  local scratch fake_cargo log env_file mismatch_env mismatch_log rc
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' RETURN
  fake_cargo="$scratch/cargo"
  log="$scratch/cargo.log"
  env_file="$scratch/nextest.env"
  mismatch_env="$scratch/mismatch.env"
  mismatch_log="$scratch/mismatch.log"

  cat >"$fake_cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$SETUP_SELFTEST_LOG"
if [ "${1:-}" != "test" ]; then
  exit 91
fi
printf '%s\n' \
  'JIT_STALE_FIXTURE_RECEIPT=/verified/receipt.json' \
  'JIT_STALE_FIXTURE_RECEIPT_SHA256=0123456789abcdef' \
  'JIT_STALE_FIXTURE_BUILT_FROM=0123456789abcdef' \
  'JIT_STALE_FIXTURE_REUSE_OBSERVATION_DIR=/observations/run' >>"$NEXTEST_ENV"
EOF
  chmod +x "$fake_cargo"

  NEXTEST=1 \
    NEXTEST_VERSION="$PINNED_NEXTEST_VERSION" \
    NEXTEST_WORKSPACE_ROOT="$PWD" \
    NEXTEST_ENV="$env_file" \
    CARGO="$fake_cargo" \
    SETUP_SELFTEST_LOG="$log" \
    run_setup
  grep -qF "test -p jit --test scratch_build $SETUP_TEST -- --exact --nocapture" "$log"

  set +e
  (
    NEXTEST=1 \
      NEXTEST_VERSION="0.9.132" \
      NEXTEST_WORKSPACE_ROOT="$PWD" \
      NEXTEST_ENV="$mismatch_env" \
      CARGO="$fake_cargo" \
      SETUP_SELFTEST_LOG="$mismatch_log" \
      run_setup
  ) >/dev/null 2>&1
  rc=$?
  set -e
  [ "$rc" -eq 2 ] || fail_environment "version mismatch self-test expected exit 2, got $rc"
  [ ! -e "$mismatch_log" ] || fail_environment "version mismatch reached Cargo"
  echo "stale-binary fixture setup self-test: PASS"
}

case "${1:-}" in
  --self-test) self_test ;;
  "") run_setup ;;
  *) fail_environment "unknown argument: $1" ;;
esac
