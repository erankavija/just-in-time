#!/usr/bin/env bash
set -euo pipefail

# scripts/benchmark-read-concurrency.sh — lock-free reads beside a continuous
# writer, before and after the read session (jit:bb596228, @/charter/D-22).
#
# Contract:
#   - Builds the base and tip `jit` binaries with
#     `cargo build --release --locked -p jit` in detached worktrees of the named
#     commits, under the host build locks, and removes the worktrees after.
#   - Generates one fixture per binary with the session-cost generator
#     (scripts/benchmark-session-cost.sh fixture mode, 665 issues) and adds one
#     unpinned document to its target issue, so the read opens a session.
#   - Waits for the 1-minute load average to fall below 2.0, then runs five
#     rounds. Each round runs, per reader count N in {0, 1, 8} and per binary
#     (base first in even rounds, tip first in odd ones), one 15 s arm on a
#     fresh copy of that binary's fixture:
#       writer   one back-to-back loop of `jit issue update <id> --priority`,
#                alternating high and normal
#       readers  N back-to-back loops of
#                `jit doc check-links --scope issue:<id> --json`
#   - Records every invocation's start, end and exit status under
#     dev/benchmarks/read-concurrency-bb596228/raw/, then writes summary.json
#     through that directory's validate.py, which owns the metrics and verdict.
#
# Environment:
#   READ_BENCH_WORK_ROOT     parent directory for worktrees, the Cargo target,
#                            binaries and fixtures (required; disk-backed). Only
#                            the harness-created child is removed on exit,
#                            except the Cargo target, which is kept for reuse.
#   READ_BENCH_BASE          base commit (default 369d35a42)
#   READ_BENCH_TIP           tip commit (default HEAD)
#   READ_BENCH_LOAD_TIMEOUT  seconds to wait for the quiet host (default 10800)
#   READ_BENCH_WINDOW_S, READ_BENCH_ROUNDS, READ_BENCH_LOAD_MAX
#                            smoke-run overrides; validate.py rejects evidence
#                            recorded with values other than the defaults.
#
# Exit codes:
#   0 — raw/ and summary.json written (the verdict is in summary.json)
#   1 — a build, fixture step or measurement failed
#   2 — environment problem (fewer than 16 logical CPUs, missing tool, existing
#       raw/, unset work root)
#   3 — the load average stayed at or above the threshold until the timeout

BASE_REF="${READ_BENCH_BASE:-369d35a42}"
TIP_REF="${READ_BENCH_TIP:-HEAD}"
LOAD_TIMEOUT="${READ_BENCH_LOAD_TIMEOUT:-10800}"
WINDOW_S="${READ_BENCH_WINDOW_S:-15}"
ROUNDS="${READ_BENCH_ROUNDS:-5}"
LOAD_MAX="${READ_BENCH_LOAD_MAX:-2.0}"
MIN_LOGICAL_CPUS=16

die() {
  echo "benchmark-read-concurrency: $1" >&2
  exit "${2:-2}"
}

for tool in cargo flock git python3 sha256sum df; do
  command -v "$tool" >/dev/null 2>&1 || die "required tool '$tool' not found on PATH"
done
[[ -n "${READ_BENCH_WORK_ROOT:-}" ]] || die "READ_BENCH_WORK_ROOT must name a disk-backed directory"
[[ "$(nproc)" -ge "$MIN_LOGICAL_CPUS" ]] ||
  die "refusing a verdict on $(nproc) logical CPUs; $MIN_LOGICAL_CPUS are required"

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) || die "not inside a git work tree"
cd "$repo_root"
EVIDENCE="$repo_root/dev/benchmarks/read-concurrency-bb596228"
[[ ! -e "$EVIDENCE/raw" ]] || die "refusing to replace existing $EVIDENCE/raw"
BASE_SHA=$(git rev-parse --verify "$BASE_REF^{commit}")
TIP_SHA=$(git rev-parse --verify "$TIP_REF^{commit}")

mkdir -p "$READ_BENCH_WORK_ROOT"
WORK_ROOT=$(realpath "$READ_BENCH_WORK_ROOT")
TARGET_DIR="$WORK_ROOT/target"
WORK=$(mktemp -d "$WORK_ROOT/run.XXXXXX")
cleanup() {
  local label
  for label in base tip; do
    [[ ! -d "$WORK/src-$label" ]] || git worktree remove --force "$WORK/src-$label"
  done
  rm -rf "$WORK"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

# build <label> <sha> — release binary at $WORK/bin/jit-<label>.
build() {
  local label="$1" sha="$2" src="$WORK/src-$1"
  git worktree add --quiet --detach "$src" "$sha"
  echo "[read-bench] building $label ($sha)" >&2
  (
    cd "$src"
    CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$TARGET_DIR" \
      flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock \
      cargo build --release --locked -p jit >&2
  ) || die "release build of $label failed" 1
  mkdir -p "$WORK/bin"
  cp "$TARGET_DIR/release/jit" "$WORK/bin/jit-$label"
  git worktree remove --force "$src"
}

# fixture <label> — the session-cost fixture plus one document on its target.
fixture() {
  local label="$1" bin="$WORK/bin/jit-$1" dir="$WORK/fixture-$1" target
  target=$(
    SESSION_BENCH_BIN="$bin" SESSION_BENCH_WORK_DIR="$WORK" \
      SESSION_BENCH_FIXTURE_DIR="$dir" "$repo_root/scripts/benchmark-session-cost.sh"
  ) || die "fixture generation for $label failed" 1
  mkdir -p "$dir/docs"
  printf '# Read benchmark\n\nA tracked document the link check reads.\n' >"$dir/docs/read-bench.md"
  (cd "$dir" && "$bin" doc add "$target" docs/read-bench.md >/dev/null) ||
    die "doc add on the $label fixture failed" 1
  (cd "$dir" && "$bin" doc check-links --scope "issue:$target" --json >/dev/null) ||
    die "the $label fixture's link check failed before measurement" 1
  printf '%s\n' "$target" >"$WORK/target-$label"
}

build base "$BASE_SHA"
build tip "$TIP_SHA"
fixture base
fixture tip

echo "[read-bench] waiting up to ${LOAD_TIMEOUT}s for the 1-minute load below $LOAD_MAX" >&2
wait_start=$SECONDS
until python3 -c 'import sys; sys.exit(float(open("/proc/loadavg").read().split()[0]) >= float(sys.argv[1]))' "$LOAD_MAX"; do
  [[ $((SECONDS - wait_start)) -lt "$LOAD_TIMEOUT" ]] ||
    die "1-minute load stayed at or above $LOAD_MAX for ${LOAD_TIMEOUT}s: $(cat /proc/loadavg)" 3
  sleep 10
done
LOAD_WAITED=$((SECONDS - wait_start))

python3 -I - "$WORK" "$EVIDENCE/raw" "$BASE_SHA" "$TIP_SHA" "$WINDOW_S" "$ROUNDS" \
  "$LOAD_MAX" "$LOAD_TIMEOUT" "$LOAD_WAITED" <<'PYEOF' || die "measurement failed" 1
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

work, raw = Path(sys.argv[1]), Path(sys.argv[2])
commits = {"base": sys.argv[3], "tip": sys.argv[4]}
window_s, rounds = int(sys.argv[5]), int(sys.argv[6])
load_max, load_timeout, load_waited = float(sys.argv[7]), int(sys.argv[8]), int(sys.argv[9])
reader_counts = [0, 1, 8]
env = {"PATH": os.environ["PATH"], "HOME": os.environ["HOME"]}
bins = {label: work / "bin" / f"jit-{label}" for label in commits}
targets = {label: (work / f"target-{label}").read_text().strip() for label in commits}


def loadavg():
    return Path("/proc/loadavg").read_text().split()[:3]


def run_loop(role, worker, argv_at, cwd, t0, deadline, samples, failures):
    index = 0
    while time.monotonic_ns() < deadline:
        start = time.monotonic_ns()
        proc = subprocess.run(argv_at(index), cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                              capture_output=True)
        end = time.monotonic_ns()
        samples.append([role, worker, start - t0, end - t0, proc.returncode])
        if proc.returncode:
            failures.append({"role": role, "worker": worker, "start_ns": start - t0,
                             "rc": proc.returncode,
                             "stdout": proc.stdout.decode(errors="replace")[:500],
                             "stderr": proc.stderr.decode(errors="replace")[:500]})
        index += 1


def run_arm(round_index, label, readers):
    cwd = work / "arm"
    shutil.rmtree(cwd, ignore_errors=True)
    shutil.copytree(work / f"fixture-{label}", cwd, symlinks=True)
    jit, target = str(bins[label]), targets[label]

    def writer(i):
        return [jit, "issue", "update", target, "--priority", ("high", "normal")[i % 2]]

    def reader(_):
        return [jit, "doc", "check-links", "--scope", f"issue:{target}", "--json"]

    samples, failures = [], []
    load_start = loadavg()
    t0 = time.monotonic_ns()
    deadline = t0 + window_s * 1_000_000_000
    loops = [("writer", 0, writer)] + [("reader", w, reader) for w in range(readers)]
    threads = [threading.Thread(target=run_loop, args=(role, worker, argv_at, cwd, t0,
                                                       deadline, samples, failures))
               for role, worker, argv_at in loops]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    samples.sort(key=lambda s: (s[2], s[0], s[1]))
    return {"round": round_index, "binary": label, "readers": readers,
            "window_ns": window_s * 1_000_000_000, "loadavg_start": load_start,
            "loadavg_end": loadavg(), "samples": samples, "failures": failures}


def fs_type(path):
    out = subprocess.run(["df", "--output=fstype", str(path)], capture_output=True, text=True)
    return out.stdout.split()[-1]


(raw / "arms").mkdir(parents=True)
host = {
    "logical_cpus": os.cpu_count(),
    "cpu_model": next((line.split(": ", 1)[1].strip()
                       for line in Path("/proc/cpuinfo").read_text().splitlines()
                       if line.startswith("model name")), "unknown"),
    "kernel": f"{platform.system()} {platform.release()}",
    "measurement_fs": fs_type(work),
    "window_s": window_s,
    "rounds": rounds,
    "reader_counts": reader_counts,
    "load_wait": {"threshold": load_max, "timeout_s": load_timeout, "waited_s": load_waited,
                  "loadavg_at_start": loadavg()},
    "binaries": {
        label: {
            "commit": commits[label],
            "sha256": hashlib.sha256(bins[label].read_bytes()).hexdigest(),
            "version": subprocess.run([str(bins[label]), "--version"], capture_output=True,
                                      text=True, check=True).stdout.strip(),
        }
        for label in commits
    },
    "fixture": {
        "generator": "scripts/benchmark-session-cost.sh (SESSION_BENCH_FIXTURE_DIR)",
        "issue_count": 665,
        "document": "docs/read-bench.md, unpinned, on the target issue",
        "targets": targets,
    },
    "writer_argv": ["jit", "issue", "update", "<id>", "--priority", "high|normal"],
    "reader_argv": ["jit", "doc", "check-links", "--scope", "issue:<id>", "--json"],
}
sequence = 0
for round_index in range(rounds):
    order = ("base", "tip") if round_index % 2 == 0 else ("tip", "base")
    for readers in reader_counts:
        for label in order:
            print(f"[read-bench] round {round_index} {label} N={readers}", file=sys.stderr)
            arm = run_arm(round_index, label, readers)
            path = raw / "arms" / f"{sequence:02d}-r{round_index}-{label}-n{readers}.json"
            path.write_text(json.dumps(arm, separators=(",", ":")) + "\n")
            sequence += 1
host["loadavg_at_end"] = loadavg()
(raw / "host.json").write_text(json.dumps(host, indent=2) + "\n")
PYEOF

python3 -I "$EVIDENCE/validate.py" --write
