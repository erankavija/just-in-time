#!/usr/bin/env bash
set -euo pipefail

# Produce the repository-owned `suite-timing-evidence` artifact consumed by
# nextest policy selection and inherently-costly-test attribution. The measured
# run deliberately has the same workspace selection as scripts/cargo-ci.sh;
# only its pinned, versioned JSON-plus reporter is different.
#
# Usage:
#   ./scripts/profile-test-suite.sh
#   ./scripts/profile-test-suite.sh --validate [ARTIFACT]
#   ./scripts/profile-test-suite.sh --self-test
#
# Environment:
#   PROFILE_TEST_SUITE_OUTPUT  Artifact path (default:
#                              dev/benchmarks/suite-profile.json)
#   CARGO_CI_BUILD_LOCK        Host-wide build lock shared with cargo-ci.sh
#
# The result contract is schema_version 1. Consumers select overrides from
# `.tests[] | select(.warm_duration_ms > LIMIT)` and use the same sorted
# records for cost attribution. `identity` is the nextest stable test identity;
# `package`, `target`, and `test_name` are its machine-split components.

readonly PINNED_NEXTEST_VERSION="0.9.133"
readonly MESSAGE_FORMAT="libtest-json-plus"
readonly MESSAGE_FORMAT_VERSION="0.1"
readonly DEFAULT_OUTPUT="dev/benchmarks/suite-profile.json"

die() {
  printf 'profile-test-suite: %s\n' "$*" >&2
  exit 2
}

ensure_pinned_nextest() {
  local probe expected
  probe=$(CARGO_INCREMENTAL=0 cargo nextest --version 2>&1 || true)
  expected="cargo-nextest $PINNED_NEXTEST_VERSION"
  if [[ "$probe" != "$expected" && "$probe" != "$expected "* ]]; then
    die "requires cargo-nextest $PINNED_NEXTEST_VERSION; observed: $probe"
  fi
}

validate_artifact() {
  local artifact="$1"
  python3 - "$artifact" <<'PYEOF'
import json
import sys

path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as stream:
        artifact = json.load(stream)
except (OSError, json.JSONDecodeError) as error:
    raise SystemExit(f"invalid suite-timing-evidence artifact: {error}")

required = {
    "contract": "suite-timing-evidence",
    "schema_version": 1,
    "artifact": "dev/benchmarks/suite-profile.json",
}
for key, value in required.items():
    if artifact.get(key) != value:
        raise SystemExit(f"invalid {key}: expected {value!r}")

method = artifact.get("method")
if not isinstance(method, dict) or method.get("cache_state") != "warm":
    raise SystemExit("invalid method.cache_state: expected 'warm'")
if method.get("runner_selection") != ["cargo", "nextest", "run", "--workspace"]:
    raise SystemExit("invalid method.runner_selection")
if method.get("nextest_version") != "0.9.133":
    raise SystemExit("invalid method.nextest_version: expected '0.9.133'")
if method.get("message_format") != "libtest-json-plus" or method.get("message_format_version") != "0.1":
    raise SystemExit("invalid versioned nextest message format")
if method.get("profiling_method") != "nextest-libtest-json-plus-v0.1":
    raise SystemExit("invalid method.profiling_method")
if method.get("warmup_runs") != 1:
    raise SystemExit("invalid method.warmup_runs: expected 1")
if method.get("timing_source") != "nextest test event exec_time in seconds":
    raise SystemExit("invalid method.timing_source")
if method.get("integer_millisecond_conversion") != "ceil(exec_time_seconds * 1000)":
    raise SystemExit("invalid method.integer_millisecond_conversion")

source = artifact.get("source")
if not isinstance(source, dict):
    raise SystemExit("source must be an object")
revision = source.get("revision")
if not isinstance(revision, str) or len(revision) != 40 or any(char not in "0123456789abcdef" for char in revision):
    raise SystemExit("invalid source.revision")
if not isinstance(source.get("dirty"), bool):
    raise SystemExit("invalid source.dirty")

tests = artifact.get("tests")
if not isinstance(tests, list) or not tests:
    raise SystemExit("tests must be a non-empty array")

identities = []
for test in tests:
    if not isinstance(test, dict):
        raise SystemExit("test record must be an object")
    for key in ("identity", "package", "target", "test_name"):
        if not isinstance(test.get(key), str) or not test[key]:
            raise SystemExit(f"test record has invalid {key}")
    if (not isinstance(test.get("warm_duration_ms"), int)
            or isinstance(test["warm_duration_ms"], bool)
            or test["warm_duration_ms"] < 0):
        raise SystemExit("test record has invalid warm_duration_ms")
    if test.get("cache_state") != "warm":
        raise SystemExit("test record cache_state must be 'warm'")
    if test.get("profiling_method") != "nextest-libtest-json-plus-v0.1":
        raise SystemExit("test record has unknown profiling_method")
    expected = f"{test['package']}::{test['target']}${test['test_name']}"
    if test["identity"] != expected:
        raise SystemExit("test identity does not agree with package/target/test_name")
    identities.append(test["identity"])

if identities != sorted(identities) or len(identities) != len(set(identities)):
    raise SystemExit("test records must be unique and sorted by identity")
if artifact.get("profiled_test_count") != len(tests):
    raise SystemExit("profiled_test_count does not match tests")
PYEOF
}

write_artifact() {
  local raw_events="$1" output="$2" revision="$3" dirty="$4"
  local output_dir stage
  output_dir=$(dirname "$output")
  mkdir -p "$output_dir"
  stage=$(mktemp "$output_dir/.suite-profile.json.XXXXXX")

  if ! python3 - "$raw_events" "$stage" "$revision" "$dirty" <<'PYEOF'
import json
import math
import sys

raw_path, stage_path, revision, dirty = sys.argv[1:]
records = {}
with open(raw_path, encoding="utf-8") as stream:
    for line in stream:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") != "test" or event.get("event") != "ok":
            continue
        identity = event.get("name")
        seconds = event.get("exec_time")
        if (not isinstance(identity, str)
                or not isinstance(seconds, (int, float))
                or isinstance(seconds, bool)
                or not math.isfinite(seconds)):
            raise SystemExit("nextest test result lacks name or exec_time")
        try:
            package, remainder = identity.split("::", 1)
            target, test_name = remainder.split("$", 1)
        except ValueError as error:
            raise SystemExit(f"unrecognised nextest test identity: {identity!r}") from error
        if not package or not target or not test_name or seconds < 0:
            raise SystemExit(f"invalid nextest test result: {identity!r}")
        if identity in records:
            raise SystemExit(f"duplicate nextest test result: {identity!r}")
        records[identity] = {
            "identity": identity,
            "package": package,
            "target": target,
            "test_name": test_name,
            "warm_duration_ms": math.ceil(seconds * 1000),
            "profiling_method": "nextest-libtest-json-plus-v0.1",
            "cache_state": "warm",
        }

tests = [records[identity] for identity in sorted(records)]
if not tests:
    raise SystemExit("nextest produced no successful profiled test events")

artifact = {
    "contract": "suite-timing-evidence",
    "schema_version": 1,
    "artifact": "dev/benchmarks/suite-profile.json",
    "source": {"revision": revision, "dirty": dirty == "true"},
    "method": {
        "runner_selection": ["cargo", "nextest", "run", "--workspace"],
        "nextest_version": "0.9.133",
        "message_format": "libtest-json-plus",
        "message_format_version": "0.1",
        "profiling_method": "nextest-libtest-json-plus-v0.1",
        "cache_state": "warm",
        "warmup_runs": 1,
        "timing_source": "nextest test event exec_time in seconds",
        "integer_millisecond_conversion": "ceil(exec_time_seconds * 1000)",
    },
    "profiled_test_count": len(tests),
    "tests": tests,
}
with open(stage_path, "w", encoding="utf-8", newline="\n") as stream:
    json.dump(artifact, stream, indent=2, sort_keys=True)
    stream.write("\n")
PYEOF
  then
    rm -f -- "$stage"
    return 1
  fi

  validate_artifact "$stage" || {
    rm -f -- "$stage"
    return 1
  }
  mv -f -- "$stage" "$output"
}

run_profile() (
  local output="$1" work raw_events warmup_events revision dirty
  ensure_pinned_nextest
  work=$(mktemp -d)
  trap 'rm -rf -- "$work"' EXIT
  raw_events="$work/nextest.jsonl"
  warmup_events="$work/warmup.log"
  revision=$(git rev-parse --verify HEAD)
  if [[ -z "$(git status --porcelain --untracked-files=normal)" ]]; then dirty=false; else dirty=true; fi

  # Prime the build and filesystem caches before the measured run. Both Cargo
  # invocations explicitly disable incremental compilation to match cargo-ci.
  if ! CARGO_CI_NO_SCCACHE=1 CARGO_INCREMENTAL=0 \
    cargo nextest run --workspace --show-progress none \
    --status-level none --final-status-level none >"$warmup_events" 2>&1; then
    cat "$warmup_events" >&2
    return 1
  fi
  if ! NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 CARGO_CI_NO_SCCACHE=1 CARGO_INCREMENTAL=0 \
    cargo nextest run --workspace --message-format "$MESSAGE_FORMAT" \
      --message-format-version "$MESSAGE_FORMAT_VERSION" --show-progress none \
      --status-level pass --final-status-level none >"$raw_events" 2>&1; then
    cat "$raw_events" >&2
    return 1
  fi

  write_artifact "$raw_events" "$output" "$revision" "$dirty"
)

self_test() (
  local work fake_bin output script_path
  work=$(mktemp -d)
  trap 'rm -rf -- "$work"' EXIT
  fake_bin="$work/bin"
  output="$work/suite-profile.json"
  script_path=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
  mkdir -p "$fake_bin"

  cat >"$fake_bin/cargo" <<'EOF'
#!/usr/bin/env bash
if [[ "${1:-}" = "nextest" && "${2:-}" = "--version" ]]; then
  printf 'cargo-nextest 0.9.133\n'
  exit 0
fi
if [[ "${1:-}" = "nextest" && "${2:-}" = "run" ]]; then
  if [[ "${CARGO_CI_NO_SCCACHE:-}" != "1" ]]; then
    printf 'expected CARGO_CI_NO_SCCACHE=1, observed: %s\n' "${CARGO_CI_NO_SCCACHE:-<unset>}" >&2
    exit 98
  fi
  if [[ "${CARGO_INCREMENTAL:-}" != "0" || " $* " != *' --workspace '* ]]; then
    printf 'unexpected nextest environment or selection: %s\n' "$*" >&2
    exit 97
  fi
  run_number=$(($(<"$PROFILE_TEST_SUITE_FAKE_CARGO_RUNS") + 1))
  printf '%s\n' "$run_number" >"$PROFILE_TEST_SUITE_FAKE_CARGO_RUNS"
  case "$run_number" in
    1)
      [[ " $* " != *' --message-format '* ]] || exit 97
      ;;
    2)
      [[ "${NEXTEST_EXPERIMENTAL_LIBTEST_JSON:-}" = "1" ]] || exit 97
      [[ " $* " = *' --message-format libtest-json-plus '* ]] || exit 97
      [[ " $* " = *' --message-format-version 0.1 '* ]] || exit 97
      printf '%s\n' \
        '{"type":"test","event":"ok","name":"jit::jit$module::test_b","exec_time":0.000001}' \
        '{"type":"test","event":"ok","name":"jit::jit$module::test_a","exec_time":0.001001}'
      ;;
    *)
      printf 'unexpected nextest run number: %s\n' "$run_number" >&2
      exit 97
      ;;
  esac
  exit 0
fi
printf 'unexpected cargo invocation: %s\n' "$*" >&2
exit 97
EOF
  cat >"$fake_bin/git" <<'EOF'
#!/usr/bin/env bash
case "${1:-} ${2:-}" in
  'rev-parse --verify') printf '0123456789abcdef0123456789abcdef01234567\n' ;;
  'status --porcelain') exit 0 ;;
  *) printf 'unexpected git invocation: %s\n' "$*" >&2; exit 97 ;;
esac
EOF
  chmod +x "$fake_bin/cargo" "$fake_bin/git"
  printf '0\n' >"$work/cargo-runs"

  PROFILE_TEST_SUITE_OUTPUT="$output" PROFILE_TEST_SUITE_FAKE_CARGO_RUNS="$work/cargo-runs" \
    PATH="$fake_bin:$PATH" "$script_path"
  validate_artifact "$output"
  python3 - "$output" <<'PYEOF'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    artifact = json.load(stream)
tests = artifact["tests"]
assert [test["identity"] for test in tests] == [
    "jit::jit$module::test_a", "jit::jit$module::test_b"
]
assert [test["warm_duration_ms"] for test in tests] == [2, 1]
PYEOF
  printf 'profile-test-suite: self-test passed\n'
)

# Keep the warmup and measured run together behind the same host-wide lock as
# cargo-ci.sh. The profile is only meaningful without a competing repository
# build consuming CPU or filesystem cache; `flock -o` avoids leaking the lock
# descriptor into Cargo descendants.
if [[ -z "${PROFILE_TEST_SUITE_LOCKED:-}" ]]; then
  build_lock="${CARGO_CI_BUILD_LOCK:-${XDG_RUNTIME_DIR:-/tmp}/cargo-ci.lock}"
  if command -v flock >/dev/null 2>&1; then
    exec env PROFILE_TEST_SUITE_LOCKED=1 flock -o "$build_lock" "$0" "$@"
  fi
  printf 'profile-test-suite: flock not found; running without host-wide build lock\n' >&2
fi

case "${1:-}" in
  "") run_profile "${PROFILE_TEST_SUITE_OUTPUT:-$DEFAULT_OUTPUT}" ;;
  --validate)
    [[ $# -le 2 ]] || die "usage: $0 --validate [ARTIFACT]"
    validate_artifact "${2:-${PROFILE_TEST_SUITE_OUTPUT:-$DEFAULT_OUTPUT}}"
    printf 'profile-test-suite: artifact is valid\n'
    ;;
  --self-test)
    [[ $# -eq 1 ]] || die "usage: $0 --self-test"
    self_test
    ;;
  *) die "usage: $0 [--validate [ARTIFACT] | --self-test]" ;;
esac
