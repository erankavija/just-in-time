#!/usr/bin/env bash
set -euo pipefail

# scripts/benchmark-session-cost.sh — reproducible session-cost measurement
# harness for the jit CLI (jit:73981310). Companion to
# scripts/benchmark-rust-build.sh; that harness times the Rust build, this one
# times the per-command session cost of the release binary against a generated
# issue corpus.
#
# Contract:
#   - Builds the release `jit` binary with real git provenance injected the same
#     way scripts/install-jit.sh does, so the artifact records the true commit,
#     dirty flag, and profile of the binary it measured.
#   - Generates ONE pristine fixture repository of a recorded issue count, then
#     times these scenarios, each with at least 3 warmup and at least 20
#     measured runs:
#       baseline        jit --version
#       single read     jit issue show <id> --json
#       read-all        jit query available --json
#       read-all        jit issue list --json
#       single mutation jit issue update <id> --priority <p> --json
#     The bulk-mutation scenario is intentionally EXCLUDED (added later, once the
#     bulk-update fix lands, so its numbers measure the fixed behavior).
#   - Reset protocol for the mutable scenario: a fresh fixture is materialized
#     (copied from the pristine template) before EVERY measured sample, so no
#     sample observes a prior sample's mutation. Warmup runs use throwaway
#     fixtures of the same shape. Read scenarios share one warm copy whose page
#     cache and per-issue sidecar locks are primed by the warmup runs.
#   - Cache-state is warm-only: cold-cache measurement needs root to drop the
#     page cache and is not attempted. Timings are comparable only across
#     artifacts that share the same `measurement_fs`.
#   - Emits ONE JSON artifact at dev/studies/perf/session-cost-<jit-commit>.json
#     — the jit commit is in the filename so history accumulates — reproducing
#     the schema of the committed session-cost-27ffbd2d.json template.
#
# Every timing figure the harness records is machine-specific. The artifact
# records machine identity (cpu, logical cpus, ram, kernel, measurement and
# real-repo filesystems) rather than asserting portability; comparisons are only
# valid within a shared measurement_fs.
#
# Environment overrides:
#   SESSION_BENCH_ISSUE_COUNT   fixture issue count (default 665)
#   SESSION_BENCH_WARMUP        warmup runs per scenario (default 3, minimum 3)
#   SESSION_BENCH_SAMPLES       measured runs per scenario (default 20, minimum 20)
#   SESSION_BENCH_OUT_DIR       artifact output directory (default dev/studies/perf)
#   SESSION_BENCH_WORK_DIR      parent dir for the disposable fixture directory;
#                               its filesystem becomes the recorded measurement_fs
#                               (default: TMPDIR or /tmp). Only the harness-created
#                               child is removed on exit.
#   SESSION_BENCH_TARGET_DIR    Cargo target directory for the release build
#                               (default: CARGO_TARGET_DIR when set, otherwise
#                               <repo>/target).
#   SESSION_BENCH_BIN           prebuilt jit binary used only by the fast
#                               harness self-test; when unset, the release
#                               binary is always built as required above.
#   CARGO_CI_BUILD_LOCK         build lock shared with cargo-ci.sh (default
#                               ${XDG_RUNTIME_DIR:-/tmp}/jit-cargo-ci.lock); held
#                               only around the release build.
#
# Exit codes:
#   0 — artifact written
#   1 — the release build failed, or a measured jit command exited non-zero
#   2 — environment problem (not a git repo, missing prerequisite, no real cargo)

ISSUE_COUNT="${SESSION_BENCH_ISSUE_COUNT:-665}"
WARMUP="${SESSION_BENCH_WARMUP:-3}"
SAMPLES="${SESSION_BENCH_SAMPLES:-20}"
OUT_DIR="${SESSION_BENCH_OUT_DIR:-dev/studies/perf}"
BUILD_LOCK="${CARGO_CI_BUILD_LOCK:-${XDG_RUNTIME_DIR:-/tmp}/jit-cargo-ci.lock}"

# Reject malformed numeric overrides rather than letting arithmetic or jq fail
# later with an unrelated message. The fixture always contains at least the
# target issue used by the single-read and single-mutation scenarios.
for setting in ISSUE_COUNT WARMUP SAMPLES; do
  value="${!setting}"
  [[ "$value" =~ ^[0-9]+$ ]] || {
    echo "ERROR: $setting must be a non-negative integer (got '$value')." >&2
    exit 2
  }
done
[[ "$ISSUE_COUNT" -ge 1 ]] || {
  echo "ERROR: SESSION_BENCH_ISSUE_COUNT must be at least 1." >&2
  exit 2
}

# The hard REQ-01 floors: at least 3 warmup and at least 20 measured runs. An
# override below the floor is clamped up rather than silently honoured.
[[ "$WARMUP" -lt 3 ]] && WARMUP=3
[[ "$SAMPLES" -lt 20 ]] && SAMPLES=20

for tool in jq python3 git df cp awk find ln rg realpath; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "ERROR: required tool '$tool' not found on PATH." >&2
    exit 2
  }
done

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || {
  echo "ERROR: not inside a git work tree." >&2
  exit 2
}
cd "$repo_root"

# Resolve the real cargo binary. Mirrors scripts/benchmark-rust-build.sh: a
# local ~/.cargo/bin/cargo debugging shim that exits 0 for every invocation
# would silently produce a non-functional "release" binary. Detect a stub via
# the canonical `cargo X.Y.Z` version probe, then fall back to a rustup
# toolchain binary.
ensure_real_cargo() {
  local probe
  probe=$(cargo --version 2>&1 || true)
  if [[ "$probe" =~ ^cargo[[:space:]][0-9]+\.[0-9]+\.[0-9]+ ]]; then
    return 0
  fi
  local tc_dir
  for tc_dir in "$HOME/.rustup/toolchains"/stable-*; do
    [[ -d "$tc_dir" && -x "$tc_dir/bin/cargo" ]] || continue
    export PATH="$tc_dir/bin:$PATH"
    probe=$(cargo --version 2>&1 || true)
    if [[ "$probe" =~ ^cargo[[:space:]][0-9]+\.[0-9]+\.[0-9]+ ]]; then
      echo "benchmark-session-cost: cargo on PATH was a stub; using $tc_dir/bin/cargo" >&2
      return 0
    fi
  done
  echo "ERROR: no real cargo on PATH and no usable rustup stable toolchain found." >&2
  echo "       cargo --version output: $probe" >&2
  exit 2
}
if [[ -z "${SESSION_BENCH_BIN:-}" ]]; then
  ensure_real_cargo
fi

# --- workspace + cleanup ------------------------------------------------------
# SESSION_BENCH_WORK_DIR is a parent, never the directory deleted by cleanup.
# This keeps a typo or a reused caller-owned directory from becoming an unsafe
# recursive-removal target.
WORK_PARENT="${SESSION_BENCH_WORK_DIR:-${TMPDIR:-/tmp}}"
mkdir -p "$WORK_PARENT"
WORK_PARENT=$(realpath "$WORK_PARENT")
WORK_BASE=$(mktemp -d "$WORK_PARENT/jit-session-bench.XXXXXX")
ARTIFACT_TMP=""
cleanup() {
  [[ -z "$ARTIFACT_TMP" ]] || rm -f -- "$ARTIFACT_TMP"
  rm -rf "$WORK_BASE"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

# --- build provenance (mirrors scripts/install-jit.sh) ------------------------
if [[ -n "${SESSION_BENCH_BIN:-}" ]]; then
  BIN=$(realpath "$SESSION_BENCH_BIN")
  echo "[session-bench] self-test binary override: $BIN" >&2
else
  TARGET_DIR="${SESSION_BENCH_TARGET_DIR:-${CARGO_TARGET_DIR:-$repo_root/target}}"
  mkdir -p "$TARGET_DIR"
  TARGET_DIR=$(realpath "$TARGET_DIR")
  GIT_HASH=$(git rev-parse HEAD)
  GIT_SHORT=$(git rev-parse --short=8 HEAD)
  if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
    GIT_DIRTY=true
  else
    GIT_DIRTY=false
  fi
  SOURCE_DATE_EPOCH=$(git show -s --format=%ct HEAD)

  echo "[session-bench] building release jit (commit $GIT_SHORT, dirty=$GIT_DIRTY)" >&2

  # Hold the shared Cargo build lock around the release build only, so we queue
  # behind (never contend with) a concurrent gate build.
  command -v flock >/dev/null 2>&1 || {
    echo "ERROR: 'flock' is required to keep benchmark build timing isolated." >&2
    exit 2
  }
  exec {LOCK_FD}>"$BUILD_LOCK"
  flock "$LOCK_FD"
  CARGO_INCREMENTAL=0 \
  CARGO_TARGET_DIR="$TARGET_DIR" \
  JIT_BUILD_GIT_HASH="$GIT_HASH" \
  JIT_BUILD_GIT_SHORT_HASH="$GIT_SHORT" \
  JIT_BUILD_GIT_DIRTY="$GIT_DIRTY" \
  SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH" \
    cargo build --release -p jit >&2 || {
    echo "ERROR: release build failed." >&2
    exit 1
  }
  flock -u "$LOCK_FD"
  exec {LOCK_FD}>&-

  BIN="$TARGET_DIR/release/jit"
fi
[[ -x "$BIN" ]] || { echo "ERROR: release binary $BIN not found." >&2; exit 1; }

# Parse the binary's own provenance report: "jit <ver> (commit <c>, dirty=<d>,
# profile <p>)". This is the authoritative identity of the measured binary.
VERSION_LINE=$("$BIN" --version)
JIT_VERSION=$(sed -E 's/^jit ([^ ]+) .*/\1/' <<<"$VERSION_LINE")
JIT_COMMIT=$(sed -E 's/.*commit ([^,]+),.*/\1/' <<<"$VERSION_LINE")
JIT_DIRTY=$(sed -E 's/.*dirty=([^,]+),.*/\1/' <<<"$VERSION_LINE")
JIT_PROFILE=$(sed -E 's/.*profile ([^)]+)\)/\1/' <<<"$VERSION_LINE")

# --- machine identity ---------------------------------------------------------
CPU_MODEL=$(awk -F': ' '/model name/{print $2; exit}' /proc/cpuinfo 2>/dev/null || echo unknown)
LOGICAL_CPUS=$(nproc)
RAM_KB=$(awk '/MemTotal/{print $2}' /proc/meminfo 2>/dev/null || echo null)
KERNEL=$(uname -sr)
REPO_REAL_FS=$(df --output=fstype "$repo_root" 2>/dev/null | tail -1 | tr -d ' ' || echo unknown)
MEASUREMENT_FS=$(df --output=fstype "$WORK_BASE" 2>/dev/null | tail -1 | tr -d ' ' || echo unknown)

# --- pristine fixture ---------------------------------------------------------
PRISTINE="$WORK_BASE/pristine"
mkdir -p "$PRISTINE"
echo "[session-bench] generating pristine fixture of $ISSUE_COUNT issues at $PRISTINE ($MEASUREMENT_FS)" >&2
(
  cd "$PRISTINE"
  "$BIN" init --quiet >/dev/null
  TARGET_ID=$("$BIN" issue create "Fixture issue 1" --type task --priority normal --orphan --quiet)
  for i in $(seq 2 "$ISSUE_COUNT"); do
    "$BIN" issue create "Fixture issue $i" --type task --priority normal --orphan --quiet >/dev/null
  done
  printf '%s\n' "$TARGET_ID" >"$WORK_BASE/target-id"
)
TARGET_ID=$(<"$WORK_BASE/target-id")
[[ -n "$TARGET_ID" && "$TARGET_ID" != null ]] || { echo "ERROR: could not resolve a fixture target id." >&2; exit 1; }
echo "[session-bench] fixture target id: $TARGET_ID" >&2

# Warm working copy shared by the read scenarios (its page cache and sidecar
# locks are primed by each scenario's warmup runs).
READ_COPY="$WORK_BASE/read"
cp -a "$PRISTINE" "$READ_COPY"

# Fixed path the mutation scenario re-materializes fresh before every sample.
MUT_COPY="$WORK_BASE/mutation"

# count_locks <dir> — number of per-issue sidecar .lock files under <dir>/.jit.
count_locks() {
  find "$1/.jit" -name '*.lock' 2>/dev/null | wc -l | tr -d ' '
}

# probe_lock_delta <argv...>
# Materialize a fresh copy, count sidecar locks before and after ONE run of the
# command, and print the delta (lock_files_created for that scenario). Not part
# of any timing.
probe_lock_delta() {
  local probe="$WORK_BASE/lockprobe"
  rm -rf "$probe"
  cp -a "$PRISTINE" "$probe"
  local before after
  before=$(count_locks "$probe")
  ( cd "$probe" && "$@" >/dev/null 2>&1 ) || {
    echo "ERROR: lock-delta probe command failed: $*" >&2
    return 1
  }
  after=$(count_locks "$probe")
  rm -rf "$probe"
  echo $((after - before))
}

# measure_scenario — run a scenario's warmup + measured loop and print the stat
# JSON {n,min_ms,median_ms,p95_ms,max_ms}. Reads a JSON spec on stdin:
#   {cwd, argv[], warmup, samples, reset_from (path or null)}
# `reset_from`, when present, is copied to `cwd` before every warmup AND
# measured sample (the mutation reset protocol).
measure_scenario() {
  local spec
  spec=$(cat)
  python3 - "$spec" <<'PYEOF'
import json
import shutil
import statistics
import subprocess
import sys
import time

spec = json.loads(sys.argv[1])
cwd = spec["cwd"]
argv = spec["argv"]
warmup = spec["warmup"]
samples = spec["samples"]
reset_from = spec.get("reset_from")


def do_reset():
    if reset_from:
        shutil.rmtree(cwd, ignore_errors=True)
        shutil.copytree(reset_from, cwd)


def run_once(phase):
    proc = subprocess.run(argv, cwd=cwd, stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL)
    if proc.returncode != 0:
        sys.stderr.write(
            f"{phase} command exited {proc.returncode}: {' '.join(argv)}\n")
        sys.exit(3)


for _ in range(warmup):
    do_reset()
    run_once("warmup")

durations = []
for _ in range(samples):
    do_reset()
    start = time.perf_counter()
    run_once("measured")
    durations.append((time.perf_counter() - start) * 1000.0)


def percentile(values, pct):
    # Linear interpolation between closest ranks (numpy's default method).
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    rank = (len(ordered) - 1) * pct
    lo = int(rank)
    hi = min(lo + 1, len(ordered) - 1)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (rank - lo)


print(json.dumps({
    "n": len(durations),
    "min_ms": round(min(durations), 2),
    "median_ms": round(statistics.median(durations), 2),
    "p95_ms": round(percentile(durations, 0.95), 2),
    "max_ms": round(max(durations), 2),
}))
PYEOF
}

# argv_to_json <argv...> — build a JSON string array from the arguments via a
# NUL-delimited stream, so a leading "--version" is never misread as a jq option
# (which `--args` positional collection does not prevent).
argv_to_json() {
  printf '%s\0' "$@" | jq -Rsc 'split("\u0000")[:-1]'
}

# scenario_stats <cwd> <reset-from|""> <argv...>
scenario_stats() {
  local cwd="$1" reset_from="$2"
  shift 2
  local argv_json
  argv_json=$(argv_to_json "$@")
  jq -cn --arg cwd "$cwd" --arg reset_from "$reset_from" \
    --argjson warmup "$WARMUP" --argjson samples "$SAMPLES" \
    --argjson argv "$argv_json" \
    '{cwd:$cwd, argv:$argv, warmup:$warmup, samples:$samples,
      reset_from:(if $reset_from=="" then null else $reset_from end)}' \
    | measure_scenario
}

echo "[session-bench] timing scenarios (warmup=$WARMUP, samples=$SAMPLES)" >&2

VERSION_STATS=$(scenario_stats "$READ_COPY" "" "$BIN" --version)
SHOW_STATS=$(scenario_stats "$READ_COPY" "" "$BIN" issue show "$TARGET_ID" --json)
QUERY_STATS=$(scenario_stats "$READ_COPY" "" "$BIN" query available --json)
LIST_STATS=$(scenario_stats "$READ_COPY" "" "$BIN" issue list --json)
UPDATE_STATS=$(scenario_stats "$MUT_COPY" "$PRISTINE" "$BIN" issue update "$TARGET_ID" --priority high --json)

VERSION_LOCKS=$(probe_lock_delta "$BIN" --version)
SHOW_LOCKS=$(probe_lock_delta "$BIN" issue show "$TARGET_ID" --json)
QUERY_LOCKS=$(probe_lock_delta "$BIN" query available --json)
LIST_LOCKS=$(probe_lock_delta "$BIN" issue list --json)
UPDATE_LOCKS=$(probe_lock_delta "$BIN" issue update "$TARGET_ID" --priority high --json)

# --- mutation syscall summary (perf stat over one mutation) -------------------
SYSCALL_JSON='null'
if command -v perf >/dev/null 2>&1; then
  rm -rf "$MUT_COPY"; cp -a "$PRISTINE" "$MUT_COPY"
  PERF_OUT="$WORK_BASE/perf.txt"
  if ( cd "$MUT_COPY" && perf stat -e task-clock,context-switches,page-faults,minor-faults \
        "$BIN" issue update "$TARGET_ID" --priority high --json >/dev/null ) 2>"$PERF_OUT"; then
    SYSCALL_JSON=$(python3 - "$PERF_OUT" <<'PYEOF'
import json
import re
import sys

text = open(sys.argv[1]).read()


def num(pattern):
    m = re.search(pattern, text)
    return float(m.group(1).replace(",", "")) if m else None


print(json.dumps({
    "command": "jit issue update <id> --priority high --json",
    "task_clock_ms": num(r"([\d.,]+)\s+msec\s+task-clock"),
    "user_s": num(r"([\d.,]+)\s+seconds user"),
    "sys_s": num(r"([\d.,]+)\s+seconds sys"),
    "page_faults": num(r"([\d.,]+)\s+page-faults"),
    "minor_faults": num(r"([\d.,]+)\s+minor-faults"),
    "context_switches": num(r"([\d.,]+)\s+context-switches"),
    "interpretation": "perf stat fields recorded for one freshly materialized single-issue mutation",
}))
PYEOF
)
  fi
fi
if [[ "$SYSCALL_JSON" == null ]]; then
  SYSCALL_JSON=$(jq -n '{
    command: "jit issue update <id> --priority high --json",
    task_clock_ms: null, user_s: null, sys_s: null,
    page_faults: null, minor_faults: null, context_switches: null,
    interpretation: "perf stat was unavailable or not permitted; syscall fields are null"
  }')
fi

# --- lock mechanism (derived, not hand-copied) --------------------------------
LOCK_SITE=$(rg -n -m1 'with_extension\("lock"\)' crates/jit/src/storage/json.rs 2>/dev/null \
  | awk -F: '{print "crates/jit/src/storage/json.rs:"$1}' || true)
if [[ -n "$LOCK_SITE" ]]; then
  LOCK_CREATOR="FileLocker::open_or_create opens the sidecar with O_CREAT"
  LOCK_LIFETIME="LockGuard::drop unlocks and removes the .lock.meta only; the .lock file persists"
  LOCK_CLEANUP="cleanup_stale_locks sweeps the claims lock directory, not .jit/issues/*.lock"
  SHOW_NOTE="reads one issue via load_issue; creates the target issue's sidecar .lock"
  QUERY_NOTE="list_issues -> load_issue per id -> one sidecar .lock per issue"
else
  LOCK_SITE="none (the measured commit has no per-issue sidecar read lock)"
  LOCK_CREATOR="not applicable; load_issue creates no per-issue sidecar"
  LOCK_LIFETIME="not applicable; no per-issue sidecar is created"
  LOCK_CLEANUP="legacy per-issue sidecars are removed during repository recovery"
  SHOW_NOTE="reads one issue without creating a per-issue sidecar lock"
  QUERY_NOTE="reads all issues without creating per-issue sidecar locks"
fi
REPO_LOCK_COUNT=$(count_locks "$repo_root")

# --- assemble the artifact ----------------------------------------------------
mkdir -p "$OUT_DIR"
ARTIFACT="$OUT_DIR/session-cost-$JIT_COMMIT.json"

FS_CAVEAT="Timings were taken on a $MEASUREMENT_FS measurement copy; the real repository lives on $REPO_REAL_FS. Filesystem-metadata costs (lock-file creation, fsync, atomic rename) are understated when measurement_fs is faster than repo_real_fs. The dominant cost (minor page faults / memory materialization) is filesystem-independent and reproduces on any backing store."
CACHE_STATE="warm (page cache primed by warmup runs); cold-cache not measured because dropping the page cache needs root"

with_stat() {
  # with_stat <name> <argv-json> <class> <stats-json> <locks> [note]
  jq -cn --arg name "$1" --argjson argv "$2" --arg class "$3" \
    --argjson stats "$4" --argjson locks "$5" --arg note "${6:-}" \
    '{name:$name, argv:$argv, class:$class, n:$stats.n, warm:true,
      min_ms:$stats.min_ms, median_ms:$stats.median_ms,
      p95_ms:$stats.p95_ms, max_ms:$stats.max_ms,
      lock_files_created:$locks}
     + (if $note=="" then {} else {note:$note} end)'
}

CMD_VERSION=$(with_stat version '["jit","--version"]' baseline "$VERSION_STATS" "$VERSION_LOCKS")
CMD_SHOW=$(with_stat issue_show_single_read '["jit","issue","show","<id>","--json"]' pure_read_single "$SHOW_STATS" "$SHOW_LOCKS" "$SHOW_NOTE")
CMD_QUERY=$(with_stat query_available '["jit","query","available","--json"]' pure_read_all "$QUERY_STATS" "$QUERY_LOCKS" "$QUERY_NOTE")
CMD_LIST=$(with_stat issue_list '["jit","issue","list","--json"]' pure_read_all "$LIST_STATS" "$LIST_LOCKS")
CMD_UPDATE=$(with_stat issue_update_mutation '["jit","issue","update","<id>","--priority","high","--json"]' mutation "$UPDATE_STATS" "$UPDATE_LOCKS" "fresh fixture materialized before every measured sample so no sample observes a prior mutation")

ARTIFACT_TMP=$(mktemp "$OUT_DIR/.session-cost.json.XXXXXX")
jq -n \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg version "$JIT_VERSION" --arg commit "$JIT_COMMIT" \
  --argjson dirty "$JIT_DIRTY" --arg profile "$JIT_PROFILE" \
  --arg cpu_model "$CPU_MODEL" --argjson logical_cpus "$LOGICAL_CPUS" \
  --argjson ram_kb "${RAM_KB:-null}" --arg kernel "$KERNEL" \
  --arg repo_real_fs "$REPO_REAL_FS" --arg measurement_fs "$MEASUREMENT_FS" \
  --arg fs_caveat "$FS_CAVEAT" \
  --argjson issue_count "$ISSUE_COUNT" \
  --argjson warmup "$WARMUP" --argjson samples "$SAMPLES" \
  --arg cache_state "$CACHE_STATE" \
  --argjson cmd_version "$CMD_VERSION" --argjson cmd_show "$CMD_SHOW" \
  --argjson cmd_query "$CMD_QUERY" --argjson cmd_list "$CMD_LIST" \
  --argjson cmd_update "$CMD_UPDATE" \
  --argjson syscall "$SYSCALL_JSON" \
  --arg lock_site "$LOCK_SITE" --arg lock_creator "$LOCK_CREATOR" \
  --arg lock_lifetime "$LOCK_LIFETIME" --arg lock_cleanup "$LOCK_CLEANUP" \
  --argjson repo_lock_count "$REPO_LOCK_COUNT" \
  '{
    schema_version: "1.0.0",
    artifact_kind: "jit-session-cost-profile",
    generated_at: $generated_at,
    jit: {version:$version, commit:$commit, dirty:$dirty, profile:$profile},
    machine: {
      cpu_model:$cpu_model, logical_cpus:$logical_cpus, ram_kb:$ram_kb,
      kernel:$kernel, repo_real_fs:$repo_real_fs, measurement_fs:$measurement_fs,
      fs_caveat:$fs_caveat
    },
    corpus: {
      issue_count:$issue_count,
      source:"freshly generated fixture: `jit init` then `jit issue create` x issue_count, each a leaf task; the mutable scenario re-copies this pristine tree before every measured sample"
    },
    method: {
      tool:"python3 time.perf_counter around subprocess.run, wall-clock milliseconds",
      warm:true,
      warmup_runs:$warmup,
      samples_per_command:$samples,
      cache_state:$cache_state,
      timing_clock:"time.perf_counter around subprocess.run, wall-clock milliseconds",
      syscall_profile_tool:"perf stat -e task-clock,context-switches,page-faults,minor-faults"
    },
    commands: [$cmd_version, $cmd_show, $cmd_query, $cmd_list, $cmd_update],
    mutation_syscall_summary: $syscall,
    lock_mechanism: {
      per_issue_lock_site:$lock_site,
      creator:$lock_creator,
      never_removed:$lock_lifetime,
      cleanup_scope:$lock_cleanup,
      current_count_in_repo:$repo_lock_count,
      gitignored:".jit/**/*.lock",
      writes_are_atomic:"issue JSON is published by temp-file plus atomic rename; repository mutations serialize on .jit/.repo-write.lock"
    }
  }' >"$ARTIFACT_TMP"

# New-file publication (@/invariant/atomic-writes): validate the complete stage,
# then use link(2) through `ln` as an atomic no-replace primitive. Both names
# are in the destination directory/filesystem. An occupied destination makes
# link fail without changing its inode or bytes; the EXIT trap removes the
# still-staged temp file on every failure path.
jq -e . "$ARTIFACT_TMP" >/dev/null || {
  echo "ERROR: staged benchmark artifact is not valid JSON: $ARTIFACT_TMP" >&2
  exit 1
}
if ! ln -T -- "$ARTIFACT_TMP" "$ARTIFACT"; then
  if [[ -e "$ARTIFACT" ]]; then
    echo "ERROR: benchmark artifact already exists; refusing to replace $ARTIFACT" >&2
  else
    echo "ERROR: could not publish benchmark artifact without replacement: $ARTIFACT" >&2
  fi
  exit 1
fi
rm -f -- "$ARTIFACT_TMP"
ARTIFACT_TMP=""
echo "[session-bench] wrote $ARTIFACT" >&2
