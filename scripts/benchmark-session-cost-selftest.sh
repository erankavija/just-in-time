#!/usr/bin/env bash
# Fast contract test for scripts/benchmark-session-cost.sh (jit:73981310).
set -euo pipefail

for tool in jq git; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "selftest: required tool '$tool' is unavailable" >&2
    exit 2
  }
done

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo "selftest: not inside a git work tree" >&2
  exit 2
}
harness="$repo_root/scripts/benchmark-session-cost.sh"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/jit-session-bench-selftest.XXXXXX")
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT

mock_jit="$scratch/jit"
cat >"$mock_jit" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$MOCK_JIT_LOG"

if [[ "${1:-}" == "--version" ]]; then
  echo 'jit 0.2.1 (commit deadbeef, dirty=false, profile release)'
  exit 0
fi

case "${1:-} ${2:-}" in
  'init '|'init --quiet')
    mkdir -p .jit/issues
    ;;
  'issue create')
    next=$(find .jit/issues -name '*.json' | wc -l | tr -d ' ')
    printf '{}\n' >".jit/issues/fixture-$next.json"
    printf 'fixture-%s\n' "$next"
    ;;
  'issue list')
    find .jit/issues -name '*.json' -exec sh -c 'touch "${1%.json}.lock"' _ {} \;
    printf '{"count":1,"issues":[{"id":"fixture-0"}]}\n'
    ;;
  'issue show')
    touch .jit/issues/fixture-0.lock
    printf '{"id":"fixture-0"}\n'
    ;;
  'query available')
    find .jit/issues -name '*.json' -exec sh -c 'touch "${1%.json}.lock"' _ {} \;
    printf '{"count":0,"issues":[]}\n'
    ;;
  'issue update')
    [[ ! -e .mutated ]] || {
      echo 'mutation fixture was reused' >&2
      exit 9
    }
    touch .mutated .jit/issues/fixture-0.lock
    printf '{"id":"fixture-0"}\n'
    ;;
  *)
    echo "unsupported mock command: $*" >&2
    exit 8
    ;;
esac
MOCK
chmod +x "$mock_jit"

out="$scratch/out"
work="$scratch/work"
export MOCK_JIT_LOG="$scratch/jit.log"
SESSION_BENCH_BIN="$mock_jit" \
SESSION_BENCH_ISSUE_COUNT=2 \
SESSION_BENCH_WARMUP=1 \
SESSION_BENCH_SAMPLES=1 \
SESSION_BENCH_OUT_DIR="$out" \
SESSION_BENCH_WORK_DIR="$work" \
  "$harness"

artifact="$out/session-cost-deadbeef.json"
[[ -f "$artifact" ]] || {
  echo "selftest: expected artifact was not written: $artifact" >&2
  exit 1
}

template="$repo_root/dev/studies/perf/session-cost-27ffbd2d.json"
jq -e --slurpfile template "$template" '
  (keys == ($template[0] | keys)) and
  ((.jit | keys) == ($template[0].jit | keys)) and
  ((.machine | keys) == ($template[0].machine | keys)) and
  ((.corpus | keys) == ($template[0].corpus | keys)) and
  ((.method | keys) == ($template[0].method | keys)) and
  ((.mutation_syscall_summary | keys) ==
    ($template[0].mutation_syscall_summary | keys)) and
  ((.lock_mechanism | keys) == ($template[0].lock_mechanism | keys)) and
  ([.commands[] | keys] == [$template[0].commands[] | keys]) and
  (.schema_version == "1.0.0") and
  (.artifact_kind == "jit-session-cost-profile") and
  (.corpus.issue_count == 2) and
  (.method.warm == true) and
  (.method.warmup_runs == 3) and
  (.method.samples_per_command == 20) and
  (.commands | length == 5) and
  ([.commands[].name] == [
    "version", "issue_show_single_read", "query_available", "issue_list",
    "issue_update_mutation"
  ]) and
  (all(.commands[];
    .n == 20 and .warm == true and
    (has("name") and has("argv") and has("class") and has("n") and
     has("min_ms") and has("median_ms") and has("p95_ms") and
     has("max_ms") and has("lock_files_created")))) and
  (all(.commands[].name; contains("bulk") | not))
' "$artifact" >/dev/null

# Three warmups and twenty measured updates all succeed only when the harness
# restores a pristine copy before each invocation; the mock exits 9 on reuse.
update_count=$(grep -c '^issue update ' "$MOCK_JIT_LOG")
[[ "$update_count" -ge 23 ]] || {
  echo "selftest: expected at least 23 mutation invocations, got $update_count" >&2
  exit 1
}

echo "benchmark-session-cost selftest: PASS"
