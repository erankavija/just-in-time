#!/usr/bin/env bash
set -euo pipefail

# scripts/benchmark-rust-build.sh — reproducible measurement harness for the
# jit Rust workspace build (jit:4e22a20d, rust-build-efficiency story jit:73482aa1).
#
# Contract:
#   - Each "clean" sample runs in a freshly created, isolated CARGO_TARGET_DIR:
#     zero-warning `cargo clippy --workspace --all-targets -- -D warnings`
#     followed by `cargo test --workspace --no-run`, each timed separately.
#   - Each "rebuild" sample begins from an equivalent completed clean build
#     (the same clippy + test --no-run sequence, run as setup in a fresh
#     isolated CARGO_TARGET_DIR), then appends one fixed, reversible,
#     comment-only probe to BENCH_PROBE_FILE, times a second
#     `cargo test --workspace --no-run` (the rebuild measurement), and restores
#     the probed file byte-for-byte, verified against the committed HEAD
#     content.
#   - Wall time, maximum resident set size, and exit status are recorded for
#     every timed command (see run_measured below).
#   - Builds serialize against other repository Cargo builds via the same
#     CARGO_CI_BUILD_LOCK convention as scripts/cargo-ci.sh and
#     scripts/verify-commit-builds.sh: a benchmark sample's cargo invocations
#     hold a blocking host-wide flock, so they queue behind (never overlap) a
#     concurrent gate build, and vice versa. The isolated per-sample
#     CARGO_TARGET_DIR already makes concurrent builds safe from a correctness
#     standpoint; the lock exists purely to keep sample timings free of
#     contention from another build competing for the same CPUs.
#   - Each sample's CARGO_TARGET_DIR is removed immediately after its
#     measurements are recorded, before the next sample starts.
#   - On the designated inventory sample (default: clean sample 1), the
#     harness additionally derives the pre-change test inventory: every
#     discoverable test case (ignored and non-ignored) with its Cargo test
#     target, from `cargo test --workspace --no-run --message-format=json`
#     plus each compiled test binary's `--list` / `--list --ignored` output,
#     together with the integration-test target count, the complete
#     target-directory byte count, and the unique active test-executable byte
#     count.
#
# Outputs (under BENCH_OUT_DIR, default dev/benchmarks/rust-build-efficiency):
#   baseline.json                  — environment + every clean/rebuild sample + medians
#   pre-change-test-inventory.json — every test case, its target and ignored status, sizes
#   raw/<sample-name>/*.log        — raw stdout+stderr of every timed command in that sample
#
# Doctests are out of scope: `cargo test --no-run` does not compile doctest
# binaries, so they contribute no test-target or executable-footprint cost and
# are not enumerated here.
#
# Environment overrides:
#   BENCH_CLEAN_SAMPLES     number of clean samples (default 3)
#   BENCH_REBUILD_SAMPLES   number of rebuild samples (default 3)
#   BENCH_OUT_DIR           output directory (default dev/benchmarks/rust-build-efficiency)
#   BENCH_TARGET_BASE       disk-backed base dir for isolated CARGO_TARGET_DIRs
#                           (default ${XDG_CACHE_HOME:-$HOME/.cache}/jit-benchmark-rust-build);
#                           each sample gets its own subdirectory, removed after
#                           that sample's measurements are recorded.
#   BENCH_INVENTORY_SAMPLE  1-based clean-sample index that derives the test
#                           inventory (default 1)
#   BENCH_PROBE_FILE        file the rebuild probe is appended to
#                           (default crates/jit/src/lib.rs)
#   CARGO_CI_BUILD_LOCK     lock path shared with cargo-ci.sh / verify-commit-builds.sh
#                           (default ${XDG_RUNTIME_DIR:-/tmp}/jit-cargo-ci.lock)
#   BENCH_NO_LOCK=1         skip the host-wide build lock (e.g. an isolated CI
#                           container that already owns the machine)
#
# Exit codes:
#   0 — all requested samples completed successfully
#   1 — a sample's clippy/build/test-compile step failed, or the probe restore
#       verification failed
#   2 — environment problem (not a git repo, dirty probe file, no real cargo,
#       or a missing jq/python3 prerequisite)
#
# Measurement methodology: maximum resident set size is read via
# getrusage(RUSAGE_CHILDREN) immediately after each command exits (see
# run_measured). This is the same mechanism GNU `time -v` uses internally and
# shares its characteristic caveat: for a multi-process build tree it is the
# high-water mark of the single largest reaped process in that tree, not a sum
# across concurrently running rustc invocations.

CLEAN_SAMPLES="${BENCH_CLEAN_SAMPLES:-3}"
REBUILD_SAMPLES="${BENCH_REBUILD_SAMPLES:-3}"
OUT_DIR="${BENCH_OUT_DIR:-dev/benchmarks/rust-build-efficiency}"
TARGET_BASE="${BENCH_TARGET_BASE:-${XDG_CACHE_HOME:-$HOME/.cache}/jit-benchmark-rust-build}"
INVENTORY_SAMPLE="${BENCH_INVENTORY_SAMPLE:-1}"
PROBE_FILE="${BENCH_PROBE_FILE:-crates/jit/src/lib.rs}"
PROBE_COMMENT="// jit-benchmark-rebuild-probe: reversible comment-only source touch (jit:4e22a20d)"
BUILD_LOCK="${CARGO_CI_BUILD_LOCK:-${XDG_RUNTIME_DIR:-/tmp}/jit-cargo-ci.lock}"

# Resolve the real cargo binary. Mirrors scripts/cargo-ci.sh: some local setups
# place a debugging shim at ~/.cargo/bin/cargo that exits 0 for every
# invocation, which would silently turn every timed step below into a false
# "success" at whatever speed the shim runs. Detect a stub via the canonical
# `cargo X.Y.Z` version-probe, then fall back to a rustup toolchain binary.
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
      echo "benchmark-rust-build: cargo on PATH was a stub; using $tc_dir/bin/cargo" >&2
      return 0
    fi
  done
  echo "ERROR: no real cargo on PATH and no usable rustup stable toolchain found." >&2
  echo "       cargo --version output: $probe" >&2
  exit 2
}

for tool in jq python3 git du cmp; do
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

ensure_real_cargo

if [[ ! -f "$PROBE_FILE" ]]; then
  echo "ERROR: BENCH_PROBE_FILE '$PROBE_FILE' does not exist." >&2
  exit 2
fi
if ! git diff --quiet -- "$PROBE_FILE" || ! git diff --cached --quiet -- "$PROBE_FILE"; then
  echo "ERROR: '$PROBE_FILE' has uncommitted changes relative to HEAD." >&2
  echo "       The rebuild-sample restore is verified against HEAD's content;" >&2
  echo "       commit or revert local changes to this file before benchmarking." >&2
  exit 2
fi

GIT_REVISION=$(git rev-parse HEAD)

mkdir -p "$TARGET_BASE" "$OUT_DIR/raw"

# --- host-wide build lock (shared with cargo-ci.sh / verify-commit-builds.sh) ---
# Acquired around each sample's cargo invocations only (not the whole script),
# so a multi-hour benchmark run does not starve concurrent gate builds between
# samples. A blocking flock queues our request rather than running concurrently
# with another lock holder, satisfying "refuses to overlap another repository
# Cargo build" (REQ-01) without needing an explicit conflict check.
LOCK_FD=""
acquire_lock() {
  [[ -n "${BENCH_NO_LOCK:-}" ]] && return 0
  command -v flock >/dev/null 2>&1 || return 0
  exec {LOCK_FD}>"$BUILD_LOCK"
  if ! flock -n "$LOCK_FD" 2>/dev/null; then
    echo "[benchmark] waiting for exclusive build lock ($BUILD_LOCK): another Cargo build is in progress" >&2
    flock "$LOCK_FD"
  fi
}
release_lock() {
  [[ -n "${BENCH_NO_LOCK:-}" ]] && return 0
  [[ -n "$LOCK_FD" ]] || return 0
  flock -u "$LOCK_FD"
  exec {LOCK_FD}>&-
  LOCK_FD=""
}

new_isolated_target_dir() {
  mktemp -d "$TARGET_BASE/target.XXXXXX"
}

# run_measured <log_file> <cmd...>
# Runs <cmd...> with combined stdout+stderr redirected to <log_file>, then
# prints ONE JSON line to stdout: {"wall_seconds":F,"max_rss_kb":N,"exit_code":N}.
# The measured command's own exit status never propagates to this wrapper's
# exit status (callers inspect the "exit_code" field); this function's own
# process always exits 0 unless the environment itself is broken.
run_measured() {
  local log_file="$1"
  shift
  python3 - "$log_file" "$@" <<'PYEOF'
import json
import resource
import subprocess
import sys
import time

log_file = sys.argv[1]
cmd = sys.argv[2:]

with open(log_file, "wb") as lf:
    start = time.monotonic()
    proc = subprocess.run(cmd, stdout=lf, stderr=subprocess.STDOUT)
    elapsed = time.monotonic() - start

ru = resource.getrusage(resource.RUSAGE_CHILDREN)
print(json.dumps({
    "wall_seconds": round(elapsed, 3),
    "max_rss_kb": ru.ru_maxrss,
    "exit_code": proc.returncode,
}))
PYEOF
}

target_dir_bytes() {
  du -sb "$1" 2>/dev/null | awk '{print $1}'
}

# generate_test_inventory <target_dir> <sample_raw_dir>
# Derives dev/benchmarks/.../pre-change-test-inventory.json from a completed
# clean build's isolated target dir, and prints
# {"target_dir_bytes":N,"unique_active_test_executable_bytes":N} to stdout for
# the caller to fold into that clean sample's baseline.json record.
generate_test_inventory() {
  local target_dir="$1" sample_raw_dir="$2"
  local list_json="$sample_raw_dir/cargo-test-list.json"

  echo "[benchmark] deriving pre-change test inventory from $target_dir" >&2
  CARGO_TARGET_DIR="$target_dir" cargo test --workspace --no-run --message-format=json \
    >"$list_json" 2>"$sample_raw_dir/cargo-test-list.stderr.log"

  python3 - "$list_json" "$target_dir" "$OUT_DIR/pre-change-test-inventory.json" "$GIT_REVISION" <<'PYEOF'
import json
import os
import subprocess
import sys

list_json_path, target_dir, out_path, git_revision = sys.argv[1:5]


def parse_test_names(text):
    names = []
    for line in text.splitlines():
        line = line.strip()
        if line.endswith(": test") or line.endswith(": benchmark"):
            names.append(line.rsplit(":", 1)[0])
    return names


targets = []
seen_executables = set()
with open(list_json_path) as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-artifact":
            continue
        executable = msg.get("executable")
        if not executable or executable in seen_executables:
            continue
        seen_executables.add(executable)
        target = msg.get("target", {})
        targets.append({
            "name": target.get("name"),
            "kind": target.get("kind", []),
            "executable": executable,
        })

for t in targets:
    exe = t["executable"]
    all_run = subprocess.run([exe, "--list"], capture_output=True, text=True, timeout=120)
    ignored_run = subprocess.run([exe, "--list", "--ignored"], capture_output=True, text=True, timeout=120)
    all_names = parse_test_names(all_run.stdout)
    ignored_names = set(parse_test_names(ignored_run.stdout))
    t["tests"] = [{"name": n, "ignored": n in ignored_names} for n in all_names]
    t["test_count"] = len(all_names)
    t["ignored_count"] = len(ignored_names)
    t["list_exit_code"] = all_run.returncode
    t["list_ignored_exit_code"] = ignored_run.returncode

integration_targets = [t for t in targets if t["kind"] == ["test"]]
total_tests = sum(t["test_count"] for t in targets)
total_ignored = sum(t["ignored_count"] for t in targets)

exe_bytes = 0
for t in targets:
    try:
        exe_bytes += os.path.getsize(t["executable"])
    except OSError:
        pass

du_out = subprocess.run(["du", "-sb", target_dir], capture_output=True, text=True)
dir_bytes = int(du_out.stdout.split()[0]) if du_out.returncode == 0 and du_out.stdout.strip() else None

result = {
    "schema_version": 1,
    "generated_at_git_revision": git_revision,
    "notes": (
        "Doctests are excluded: `cargo test --no-run` does not compile doctest "
        "binaries. 'kind' mirrors Cargo's target.kind (e.g. [\"lib\"], [\"bin\"], "
        "[\"test\"] for a tests/*.rs integration target). Ignored status is the "
        "set difference between `<exe> --list` and `<exe> --list --ignored`."
    ),
    "targets": targets,
    "totals": {
        "test_target_count": len(targets),
        "integration_test_target_count": len(integration_targets),
        "test_case_count": total_tests,
        "ignored_test_case_count": total_ignored,
    },
    "target_dir_bytes": dir_bytes,
    "unique_active_test_executable_bytes": exe_bytes,
}

with open(out_path, "w") as out:
    json.dump(result, out, indent=2)
    out.write("\n")

print(json.dumps({
    "target_dir_bytes": dir_bytes,
    "unique_active_test_executable_bytes": exe_bytes,
    "integration_test_target_count": len(integration_targets),
    "test_case_count": total_tests,
    "ignored_test_case_count": total_ignored,
}))
PYEOF
}

CLEAN_JSONL="$OUT_DIR/raw/clean-samples.jsonl"
REBUILD_JSONL="$OUT_DIR/raw/rebuild-samples.jsonl"
: >"$CLEAN_JSONL"
: >"$REBUILD_JSONL"

run_clean_sample() {
  local idx="$1"
  local name="clean-$idx"
  local sample_raw_dir="$OUT_DIR/raw/$name"
  mkdir -p "$sample_raw_dir"

  local target_dir
  target_dir=$(new_isolated_target_dir)
  echo "[benchmark] clean sample $idx/$CLEAN_SAMPLES: target dir $target_dir" >&2

  acquire_lock
  local clippy_meas test_meas
  clippy_meas=$(CARGO_TARGET_DIR="$target_dir" run_measured "$sample_raw_dir/clippy.log" \
    cargo clippy --workspace --all-targets -- -D warnings)
  test_meas=$(CARGO_TARGET_DIR="$target_dir" run_measured "$sample_raw_dir/test-no-run.log" \
    cargo test --workspace --no-run)

  local inventory_meas="{}"
  if [[ "$idx" -eq "$INVENTORY_SAMPLE" ]]; then
    inventory_meas=$(generate_test_inventory "$target_dir" "$sample_raw_dir")
  fi
  release_lock

  local dir_bytes
  dir_bytes=$(target_dir_bytes "$target_dir")
  rm -rf "$target_dir"

  local clippy_exit test_exit success
  clippy_exit=$(echo "$clippy_meas" | jq -r .exit_code)
  test_exit=$(echo "$test_meas" | jq -r .exit_code)
  success=$([[ "$clippy_exit" -eq 0 && "$test_exit" -eq 0 ]] && echo true || echo false)

  jq -cn \
    --argjson sample "$idx" \
    --argjson clippy "$clippy_meas" \
    --argjson test_no_run "$test_meas" \
    --argjson target_dir_bytes "${dir_bytes:-null}" \
    --argjson inventory "$inventory_meas" \
    --argjson success "$success" \
    '{sample: $sample, clippy: $clippy, test_no_run: $test_no_run, target_dir_bytes: $target_dir_bytes, inventory: $inventory, success: $success}' \
    >>"$CLEAN_JSONL"

  if [[ "$success" != "true" ]]; then
    echo "ERROR: clean sample $idx failed (clippy exit $clippy_exit, test-no-run exit $test_exit)." >&2
    echo "       see $sample_raw_dir/clippy.log and $sample_raw_dir/test-no-run.log" >&2
    exit 1
  fi
  echo "[benchmark] clean sample $idx/$CLEAN_SAMPLES done: clippy=$(echo "$clippy_meas" | jq -r .wall_seconds)s test-no-run=$(echo "$test_meas" | jq -r .wall_seconds)s" >&2
}

run_rebuild_sample() {
  local idx="$1"
  local name="rebuild-$idx"
  local sample_raw_dir="$OUT_DIR/raw/$name"
  mkdir -p "$sample_raw_dir"

  local target_dir
  target_dir=$(new_isolated_target_dir)
  echo "[benchmark] rebuild sample $idx/$REBUILD_SAMPLES: target dir $target_dir (setup clean build)" >&2

  acquire_lock
  local setup_clippy_meas setup_test_meas
  setup_clippy_meas=$(CARGO_TARGET_DIR="$target_dir" run_measured "$sample_raw_dir/setup-clippy.log" \
    cargo clippy --workspace --all-targets -- -D warnings)
  setup_test_meas=$(CARGO_TARGET_DIR="$target_dir" run_measured "$sample_raw_dir/setup-test-no-run.log" \
    cargo test --workspace --no-run)
  release_lock

  local setup_clippy_exit setup_test_exit
  setup_clippy_exit=$(echo "$setup_clippy_meas" | jq -r .exit_code)
  setup_test_exit=$(echo "$setup_test_meas" | jq -r .exit_code)
  if [[ "$setup_clippy_exit" -ne 0 || "$setup_test_exit" -ne 0 ]]; then
    rm -rf "$target_dir"
    echo "ERROR: rebuild sample $idx setup (equivalent clean build) failed (clippy exit $setup_clippy_exit, test-no-run exit $setup_test_exit)." >&2
    echo "       see $sample_raw_dir/setup-clippy.log and $sample_raw_dir/setup-test-no-run.log" >&2
    exit 1
  fi

  # Capture the original file (cp), apply the fixed reversible probe, time the
  # incremental rebuild, then restore via mv and verify against HEAD via cmp.
  local backup
  backup=$(mktemp "$TARGET_BASE/probe-backup.XXXXXX")
  cp "$PROBE_FILE" "$backup"
  printf '\n%s\n' "$PROBE_COMMENT" >>"$PROBE_FILE"

  echo "[benchmark] rebuild sample $idx/$REBUILD_SAMPLES: probe applied, timing incremental rebuild" >&2
  acquire_lock
  local rebuild_meas
  rebuild_meas=$(CARGO_TARGET_DIR="$target_dir" run_measured "$sample_raw_dir/rebuild-test-no-run.log" \
    cargo test --workspace --no-run)
  release_lock

  mv "$backup" "$PROBE_FILE"
  local restore_ok=true
  if ! cmp -s <(git show "HEAD:$PROBE_FILE") "$PROBE_FILE"; then
    restore_ok=false
  fi

  local dir_bytes
  dir_bytes=$(target_dir_bytes "$target_dir")
  rm -rf "$target_dir"

  if [[ "$restore_ok" != "true" ]]; then
    echo "ERROR: rebuild sample $idx: $PROBE_FILE did not restore byte-for-byte against HEAD." >&2
    exit 1
  fi

  local rebuild_exit success
  rebuild_exit=$(echo "$rebuild_meas" | jq -r .exit_code)
  success=$([[ "$rebuild_exit" -eq 0 ]] && echo true || echo false)

  jq -cn \
    --argjson sample "$idx" \
    --argjson setup_clippy "$setup_clippy_meas" \
    --argjson setup_test_no_run "$setup_test_meas" \
    --argjson rebuild_test_no_run "$rebuild_meas" \
    --argjson target_dir_bytes "${dir_bytes:-null}" \
    --argjson probe_restored_verified "$restore_ok" \
    --argjson success "$success" \
    '{sample: $sample, setup_clippy: $setup_clippy, setup_test_no_run: $setup_test_no_run, rebuild_test_no_run: $rebuild_test_no_run, target_dir_bytes: $target_dir_bytes, probe_restored_verified: $probe_restored_verified, success: $success}' \
    >>"$REBUILD_JSONL"

  if [[ "$success" != "true" ]]; then
    echo "ERROR: rebuild sample $idx failed (rebuild test-no-run exit $rebuild_exit)." >&2
    echo "       see $sample_raw_dir/rebuild-test-no-run.log" >&2
    exit 1
  fi
  echo "[benchmark] rebuild sample $idx/$REBUILD_SAMPLES done: rebuild=$(echo "$rebuild_meas" | jq -r .wall_seconds)s" >&2

  if [[ -n "$(git status --porcelain -- "$PROBE_FILE")" ]]; then
    echo "ERROR: $PROBE_FILE shows as modified in git status after restore." >&2
    exit 1
  fi
}

for i in $(seq 1 "$CLEAN_SAMPLES"); do
  run_clean_sample "$i"
done

for i in $(seq 1 "$REBUILD_SAMPLES"); do
  run_rebuild_sample "$i"
done

echo "[benchmark] all samples complete; assembling baseline.json" >&2

CLEAN_ARRAY=$(jq -s '.' "$CLEAN_JSONL")
REBUILD_ARRAY=$(jq -s '.' "$REBUILD_JSONL")

RUSTC_VVERBOSE=$(rustc -vV)
HOST_TRIPLE=$(echo "$RUSTC_VVERBOSE" | awk -F': ' '/^host:/{print $2}')
OS_PRETTY=$( (grep -m1 '^PRETTY_NAME=' /etc/os-release 2>/dev/null || echo 'PRETTY_NAME="unknown"') | cut -d= -f2 | tr -d '"')
KERNEL=$(uname -sr)
CPU_COUNT=$(nproc)
MEM_TOTAL_BYTES=$(awk '/MemTotal/{print $2*1024}' /proc/meminfo 2>/dev/null || echo null)
CARGO_VERSION=$(cargo --version)
RUSTC_VERSION=$(rustc --version)

if [[ -f .cargo/config.toml || -f "$HOME/.cargo/config.toml" ]]; then
  LINKER_DESC="explicit .cargo/config.toml present; inspect [target.*.linker] there"
else
  LINKER_DESC="cargo default (no linker override in .cargo/config.toml): $(cc --version | head -1) invoking $(ld --version | head -1)"
fi

CARGO_RUST_ENV=$(env | grep -E '^(CARGO|RUST)[A-Z_]*=' | jq -Rn '[inputs | split("=") | {(.[0]): (.[1:] | join("="))}] | add // {}')

jq -n \
  --argjson clean_samples "$CLEAN_ARRAY" \
  --argjson rebuild_samples "$REBUILD_ARRAY" \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg git_revision "$GIT_REVISION" \
  --arg os "$OS_PRETTY" \
  --arg kernel "$KERNEL" \
  --argjson cpu_count "$CPU_COUNT" \
  --argjson memory_bytes "$MEM_TOTAL_BYTES" \
  --arg cargo_version "$CARGO_VERSION" \
  --arg rustc_version "$RUSTC_VERSION" \
  --arg rustc_host_triple "$HOST_TRIPLE" \
  --arg linker "$LINKER_DESC" \
  --argjson cargo_rust_env "$CARGO_RUST_ENV" \
  --arg build_lock "$BUILD_LOCK" \
  '
  def median: sort | if (length % 2) == 1 then .[length/2 | floor]
              else (.[length/2 - 1] + .[length/2]) / 2 end;
  {
    schema_version: 1,
    generated_at: $generated_at,
    environment: {
      git_revision: $git_revision,
      os: $os,
      kernel: $kernel,
      cpu_count: $cpu_count,
      memory_bytes: $memory_bytes,
      cargo_version: $cargo_version,
      rustc_version: $rustc_version,
      rustc_host_triple: $rustc_host_triple,
      linker: $linker,
      cargo_rust_env: $cargo_rust_env
    },
    measurement_methodology: {
      max_rss_source: "getrusage(RUSAGE_CHILDREN) high-water mark after each command exits; equivalent to GNU time -v Maximum resident set size; reflects the largest single reaped process in the tree, not a sum across concurrent rustc invocations.",
      build_lock_path: $build_lock,
      isolation: "each sample runs in a freshly created CARGO_TARGET_DIR, removed immediately after its measurements are recorded."
    },
    clean_samples: $clean_samples,
    rebuild_samples: $rebuild_samples,
    medians: {
      clean_clippy_wall_seconds: ($clean_samples | map(.clippy.wall_seconds) | median),
      clean_test_no_run_wall_seconds: ($clean_samples | map(.test_no_run.wall_seconds) | median),
      clean_total_wall_seconds: ($clean_samples | map(.clippy.wall_seconds + .test_no_run.wall_seconds) | median),
      rebuild_wall_seconds: ($rebuild_samples | map(.rebuild_test_no_run.wall_seconds) | median)
    }
  }' >"$OUT_DIR/baseline.json"

echo "[benchmark] wrote $OUT_DIR/baseline.json" >&2
echo "[benchmark] wrote $OUT_DIR/pre-change-test-inventory.json" >&2
