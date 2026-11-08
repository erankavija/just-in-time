#!/usr/bin/env bash
set -euo pipefail

# scripts/benchmark-cost-evidence.sh — the four REQ-22 cost claims of the
# one-mutation-path epic, before and after (jit:45d0a971, @/charter/D-17,
# @/charter/D-19).
#
# Usage:
#   benchmark-cost-evidence.sh <tip-binary>   base and tip; records raw/tip/
#   benchmark-cost-evidence.sh --base-only    base alone; records raw/base/
#
# Contract:
#   - Builds the base `jit` from commit 369d35a42 with
#     `cargo build --release --locked -p jit` in a `git archive` extraction,
#     under the host build locks. The tip is the binary named on the command line.
#   - Generates every fixture with the binary it measures, then waits for the
#     1-minute load average to fall below 2.0 and runs five rounds. Each round
#     runs every point below once per binary, alternating which binary goes first,
#     each run on a fresh copy of the point's pristine fixture at one fixed path:
#       bulk     timed `jit issue update --filter label:bench:match
#                --state in_progress --json`, M in {10, 160} matches among N in
#                {500, 2000} ready issues
#       archive  timed `jit archive candidates --json` over C in {10, 40} done
#                epics, each over one done task, beside 30 open epics of 50 tasks,
#                with the event log replayed to the measured repository's 18482
#                lines per 1589 issues; then the same run under strace
#       dep      timed `jit dep add <a> <b>` over the minimal declarations of
#                `jit init` and over 2,000 declared namespaces plus 300 rules
#       profile  `jit profile add` of one 48-file package whose assets sit in four
#                nested directories, under strace
#   - strace (`-f -y`, openat/openat2/open/creat and fsync/fdatasync) records,
#     per traced run, the opens of `.jit/config.toml` and every directory open
#     and sync, classified while the run's tree still exists.
#   - Writes raw/<label>/ under dev/benchmarks/cost-evidence-45d0a971/, then runs
#     that directory's validate.py --write, which owns every metric, ratio and
#     verdict and writes summary.json.
#
# Environment:
#   COST_BENCH_BUILD_ROOT    base extraction and Cargo target (default
#                            ${TMPDIR:-/tmp}/jit-cost-evidence-build; the target
#                            is kept for reuse)
#   COST_BENCH_WORK_ROOT     fixtures and runs (default /dev/shm/jit-cost-evidence);
#                            only the harness-created child is removed on exit
#   COST_BENCH_LOAD_TIMEOUT  seconds to wait for the quiet host (default 10800)
#   COST_BENCH_SAMPLES, COST_BENCH_LOAD_MAX, COST_BENCH_RAW
#                            smoke-run overrides (rounds, default 5; load
#                            threshold, default 2.0; output directory, which
#                            skips summary.json); validate.py rejects evidence
#                            recorded with other values
#
# Exit codes:
#   0 — raw/<label>/ written (the verdict is in summary.json)
#   1 — a build, fixture step or measurement failed
#   2 — usage or environment problem (missing tool, existing output)
#   3 — the load average stayed at or above the threshold until the timeout

BASE_REF=369d35a42
SAMPLES="${COST_BENCH_SAMPLES:-5}"
LOAD_MAX="${COST_BENCH_LOAD_MAX:-2.0}"
LOAD_TIMEOUT="${COST_BENCH_LOAD_TIMEOUT:-10800}"

die() {
  echo "benchmark-cost-evidence: $1" >&2
  exit "${2:-2}"
}

case "${1:-}" in
  --base-only) [[ $# -eq 1 ]] || die "usage: $0 <tip-binary> | --base-only"
    LABEL=base TIP_BIN= ;;
  ''|-*) die "usage: $0 <tip-binary> | --base-only" ;;
  *) [[ $# -eq 1 ]] || die "usage: $0 <tip-binary> | --base-only"
    LABEL=tip TIP_BIN=$(realpath "$1") || die "tip binary not found: $1"
    [[ -x "$TIP_BIN" ]] || die "tip binary is not executable: $TIP_BIN" ;;
esac

command -v strace >/dev/null 2>&1 ||
  die "strace is required to count opens and syncs; install it (e.g. 'sudo pacman -S strace' or 'sudo apt-get install strace')"
strace -f -o /dev/null true 2>/dev/null ||
  die "strace cannot trace a child here; allow ptrace (kernel.yama.ptrace_scope) and retry"
for tool in cargo flock git python3 sha256sum tar; do
  command -v "$tool" >/dev/null 2>&1 || die "required tool '$tool' not found on PATH"
done

repo_root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel 2>/dev/null) ||
  die "not inside a git work tree"
EVIDENCE="$repo_root/dev/benchmarks/cost-evidence-45d0a971"
RAW="${COST_BENCH_RAW:-$EVIDENCE/raw/$LABEL}"
[[ ! -e "$RAW" ]] || die "refusing to replace existing $RAW"
BASE_SHA=$(git -C "$repo_root" rev-parse --verify "$BASE_REF^{commit}") ||
  die "base commit $BASE_REF is not in this repository"

BUILD_ROOT="${COST_BENCH_BUILD_ROOT:-${TMPDIR:-/tmp}/jit-cost-evidence-build}"
WORK_ROOT="${COST_BENCH_WORK_ROOT:-/dev/shm/jit-cost-evidence}"
mkdir -p "$BUILD_ROOT" "$WORK_ROOT"
BUILD_ROOT=$(realpath "$BUILD_ROOT")
WORK=$(mktemp -d "$(realpath "$WORK_ROOT")/run.XXXXXX")
cleanup() { rm -rf "$WORK" "$BUILD_ROOT/src"; }
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

echo "[cost-bench] building base ($BASE_SHA)" >&2
rm -rf "$BUILD_ROOT/src" && mkdir -p "$BUILD_ROOT/src" "$WORK/bin"
git -C "$repo_root" archive "$BASE_SHA" | tar -x -C "$BUILD_ROOT/src" ||
  die "extracting $BASE_SHA failed" 1
(
  cd "$BUILD_ROOT/src"
  CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$BUILD_ROOT/target" \
    flock -o /tmp/jit-ci-turnstile.lock flock -o -s /tmp/jit-full-ci.lock \
    cargo build --release --locked -p jit >&2
) || die "release build of the base failed" 1
cp "$BUILD_ROOT/target/release/jit" "$WORK/bin/jit-base"
[[ -z "$TIP_BIN" ]] || cp "$TIP_BIN" "$WORK/bin/jit-tip"

python3 -I - "$WORK" "$RAW" "$BASE_SHA" "$LABEL" "$SAMPLES" "$LOAD_MAX" "$LOAD_TIMEOUT" \
  "$(git -C "$repo_root" rev-parse HEAD)" <<'PYEOF' || exit $?
import hashlib
import json
import math
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time
import uuid
from pathlib import Path

work, raw = Path(sys.argv[1]), Path(sys.argv[2])
base_sha, label, samples = sys.argv[3], sys.argv[4], int(sys.argv[5])
load_max, load_timeout, harness_head = float(sys.argv[6]), int(sys.argv[7]), sys.argv[8]
binaries = ["base"] if label == "base" else ["base", "tip"]
bins = {b: work / "bin" / f"jit-{b}" for b in binaries}
env = {"PATH": os.environ["PATH"], "HOME": os.environ["HOME"], "LC_ALL": "C"}
run_dir = work / "run"

BULK = [(n, m) for n in (500, 2000) for m in (10, 160)]
ARCHIVE = (10, 40)
ARCHIVE_OPEN_EPICS, ARCHIVE_TASKS_PER_OPEN_EPIC = 30, 50
EVENT_LINES, EVENT_ISSUES = 18482, 1589
NAMESPACES, RULES = 2000, 300
PACKAGE_DIRS = ["assets/d1", "assets/d1/d2", "assets/d1/d2/d3", "assets/d1/d2/d3/d4"]
PACKAGE_FILES = 48
TRACED = ["openat", "openat2", "open", "creat", "fsync", "fdatasync"]


def fail(message):
    print(f"benchmark-cost-evidence: {message}", file=sys.stderr)
    sys.exit(1)


def jit(binary, cwd, *args, check=True):
    proc = subprocess.run([str(bins[binary]), *args], cwd=cwd, env=env,
                          stdin=subprocess.DEVNULL, capture_output=True, text=True)
    if check and proc.returncode:
        fail(f"{binary}: jit {' '.join(args)} exited {proc.returncode}: {proc.stderr[:400]}")
    return proc


def loadavg():
    return Path("/proc/loadavg").read_text().split()[:3]


# --- fixtures, each generated by the binary it measures ---------------------

BENCH_NAMESPACE = ('\n[namespaces.bench]\ndescription = "Benchmark grouping"\n'
                   'unique = false\n')
HIERARCHY = ('\n[type_hierarchy]\ntypes = { epic = 1, task = 2 }\n\n'
             '[namespaces.type]\ndescription = "Issue type"\nunique = true\n')


def init(binary, path, config_tail=""):
    path.mkdir(parents=True)
    jit(binary, path, "init")
    if config_tail:
        with (path / ".jit" / "config.toml").open("a") as config:
            config.write(config_tail)
        jit(binary, path, "validate", "--fix")


def batch_create(binary, path, entries):
    batch = path.parent / f"{path.name}.batch.json"
    batch.write_text(json.dumps(entries))
    created = json.loads(jit(binary, path, "issue", "batch-create", "--from-json",
                             str(batch), "--json").stdout)
    batch.unlink()
    return created.get("ids", created)


def event_lines(path):
    return sum(1 for line in (path / ".jit" / "events.jsonl").open("rb") if line.strip())


def replay_events(path, target_lines):
    """Grow the event log to `target_lines` by replaying its own lines under fresh ids."""
    log = path / ".jit" / "events.jsonl"
    lines = [line for line in log.read_bytes().split(b"\n") if line.strip()]
    with log.open("ab") as out:
        written = len(lines)
        while written < target_lines:
            for line in lines[: target_lines - written]:
                record = json.loads(line)
                record["id"] = str(uuid.uuid4())
                out.write(json.dumps(record, separators=(",", ":")).encode() + b"\n")
                written += 1


def bulk_fixture(binary, path, n, m):
    init(binary, path, BENCH_NAMESPACE)
    batch_create(binary, path, [
        {"key": f"i{i}", "title": f"bulk issue {i}",
         "labels": ["bench:match" if i < m else "bench:other"]}
        for i in range(n)])
    return {"issues": n, "matches": m}


def archive_fixture(binary, path, c):
    init(binary, path, HIERARCHY + BENCH_NAMESPACE)
    entries = []
    for e in range(ARCHIVE_OPEN_EPICS):
        tasks = [f"o{e}t{t}" for t in range(ARCHIVE_TASKS_PER_OPEN_EPIC)]
        entries += [{"key": k, "title": f"open task {k}", "labels": ["type:task"]} for k in tasks]
        entries.append({"key": f"o{e}", "title": f"open epic {e}", "labels": ["type:epic"],
                        "depends_on": tasks})
    for e in range(c):
        entries.append({"key": f"c{e}t", "title": f"closed task {e}",
                        "labels": ["type:task", "bench:closed-task"]})
        entries.append({"key": f"c{e}", "title": f"closed epic {e}",
                        "labels": ["type:epic", "bench:closed-epic"], "depends_on": [f"c{e}t"]})
    batch_create(binary, path, entries)
    for value, count in (("closed-task", c), ("closed-epic", c)):
        result = json.loads(jit(binary, path, "issue", "update", "--filter",
                                f"label:bench:{value}", "--state", "done", "--json").stdout)
        if len(result["modified"]) != count or result["errors"]:
            fail(f"{binary}: closing {value} modified {len(result['modified'])} of {count}")
    issues = len(entries)
    replay_events(path, math.ceil(issues * EVENT_LINES / EVENT_ISSUES))
    jit(binary, path, "validate")
    return {"issues": issues, "terminal_containers": c, "event_lines": event_lines(path)}


def dep_fixture(binary, path, inflated):
    init(binary, path)
    if inflated:
        with (path / ".jit" / "config.toml").open("a") as config:
            for i in range(NAMESPACES):
                config.write(f'\n[namespaces.ns{i:04d}]\ndescription = "Benchmark namespace '
                             f'{i}"\nunique = false\nexamples = ["ns{i:04d}:value"]\n')
        with (path / ".jit" / "rules.toml").open("a") as rules:
            for i in range(RULES):
                rules.write(f'\n[[rules]]\nname = "bench-pattern-{i:03d}"\n'
                            f'when = {{ label = "ns{i:04d}:*" }}\nseverity = "warn"\n'
                            f'assert = {{ label-value-pattern = {{ namespace = "ns{i:04d}", '
                            f"regex = '^[a-z0-9-]+$' }} }}\n")
        jit(binary, path, "validate", "--fix")
    ids = batch_create(binary, path, [{"key": "a", "title": "dependent"},
                                      {"key": "b", "title": "dependency"}])
    return {"namespaces": NAMESPACES if inflated else 0, "added_rules": RULES if inflated else 0,
            "from": ids["a"], "to": ids["b"]}


def profile_fixture(binary, path):
    init(binary, path)
    source = path / "bench-package"
    manifest = ['[profile]', 'manifest-version = 2', 'id = "bench-package"',
                'version = "1.0.0"', 'compatible-jit = "*"', '']
    counts = [PACKAGE_FILES - 1 - 12 * (len(PACKAGE_DIRS) - 1)] + [12] * (len(PACKAGE_DIRS) - 1)
    for directory, count in zip(PACKAGE_DIRS, counts):
        (source / directory).mkdir(parents=True)
        for i in range(count):
            rel = f"{directory}/file-{i:02d}.md"
            (source / rel).write_text(f"# Benchmark file {rel}\n")
            manifest += ['[[asset]]', f'source = "{rel}"',
                         f'target = "bench/{rel.removeprefix("assets/")}"',
                         'executable = false', 'template = false', '']
    (source / "manifest.toml").write_text("\n".join(manifest))
    archive = work / f"package-{binary}.jitprof"
    jit(binary, path, "profile", "pack", "--source", "bench-package", "--output", str(archive))
    shutil.rmtree(source)
    return {"package_files": PACKAGE_FILES, "nested_directories": len(PACKAGE_DIRS),
            "archive": str(archive),
            "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}


fixtures = {}
for binary in binaries:
    print(f"[cost-bench] generating {binary} fixtures", file=sys.stderr)
    root = work / "fixtures" / binary
    points = {}
    for n, m in BULK:
        points[f"bulk/N{n}-M{m}"] = bulk_fixture(binary, root / f"bulk-N{n}-M{m}", n, m)
    for c in ARCHIVE:
        points[f"archive/C{c}"] = archive_fixture(binary, root / f"archive-C{c}", c)
    for kind in ("minimal", "inflated"):
        points[f"dep/{kind}"] = dep_fixture(binary, root / f"dep-{kind}", kind == "inflated")
    points["profile/package48"] = profile_fixture(binary, root / "profile-package48")
    fixtures[binary] = points


# --- measurement --------------------------------------------------------------

def fresh_copy(binary, point):
    shutil.rmtree(run_dir, ignore_errors=True)
    shutil.copytree(work / "fixtures" / binary / point.replace("/", "-"), run_dir, symlinks=True)


def argv_for(binary, point):
    scenario, _, name = point.partition("/")
    fixture = fixtures[binary][point]
    if scenario == "bulk":
        return ["issue", "update", "--filter", "label:bench:match", "--state", "in_progress",
                "--json"]
    if scenario == "archive":
        return ["archive", "candidates", "--json"]
    if scenario == "dep":
        return ["dep", "add", fixture["from"], fixture["to"]]
    return ["profile", "add", "--archive", fixture["archive"], "--destination",
            "packages/bench-package"]


def check(binary, point, proc):
    """The run did the work the point measures; a mismatch aborts the harness."""
    scenario = point.partition("/")[0]
    fixture = fixtures[binary][point]
    if proc.returncode:
        fail(f"{binary} {point} exited {proc.returncode}: {proc.stderr[:400]}")
    if scenario == "bulk":
        result = json.loads(proc.stdout)
        if len(result["modified"]) != fixture["matches"] or result["errors"]:
            fail(f"{binary} {point} modified {len(result['modified'])} of {fixture['matches']}")
    elif scenario == "archive":
        if json.loads(proc.stdout)["count"] != fixture["terminal_containers"]:
            fail(f"{binary} {point} reported the wrong candidate count")
    elif scenario == "profile":
        placed = [p for p in (run_dir / "packages" / "bench-package").rglob("*") if p.is_file()]
        if len(placed) != PACKAGE_FILES:
            fail(f"{binary} {point} placed {len(placed)} of {PACKAGE_FILES} files")


def timed(binary, point):
    fresh_copy(binary, point)
    start = time.perf_counter_ns()
    proc = subprocess.run([str(bins[binary]), *argv_for(binary, point)], cwd=run_dir, env=env,
                          stdin=subprocess.DEVNULL, capture_output=True, text=True)
    wall_ns = time.perf_counter_ns() - start
    check(binary, point, proc)
    return wall_ns


DECORATED = re.compile(r"^\d+\s+(\w+)\((.*)\)\s+=\s+(-?\d+)(?:<(.*)>)?")
DIRFD = re.compile(r"^-?\d+<([^>]*)>")


def classify(trace_path):
    """Rows `<call> <dir|file> <inside|outside> <path>` for each successful open and sync,
    judged while the run's tree exists; a path is a directory when it is one now, was
    opened with O_DIRECTORY, or served as a *at call's directory descriptor."""
    calls = []
    dirfds = set()
    for line in trace_path.read_text(errors="replace").splitlines():
        match = DECORATED.match(line)
        if not match:
            continue
        call, args, result, returned = match.groups()
        used = DIRFD.match(args)
        if used and call in ("openat", "openat2"):
            dirfds.add(used.group(1))
        if int(result) < 0:
            continue
        if call in ("fsync", "fdatasync"):
            calls.append((call, used.group(1) if used else "?", False))
        elif returned:
            calls.append(("open", returned, "O_DIRECTORY" in args))
    root = str(run_dir)
    rows = []
    for call, path, directory_flag in calls:
        kind = "dir" if directory_flag or path in dirfds or os.path.isdir(path) else "file"
        inside = path == root or path.startswith(root + "/")
        rel = os.path.relpath(path, root) if inside else path
        rows.append(f"{call}\t{kind}\t{'inside' if inside else 'outside'}\t{rel}")
    return rows


def traced(binary, point, round_index):
    fresh_copy(binary, point)
    trace = work / "trace.txt"
    proc = subprocess.run(["strace", "-f", "-y", "-qq", "-e", f"trace={','.join(TRACED)}",
                           "-o", str(trace), str(bins[binary]), *argv_for(binary, point)],
                          cwd=run_dir, env=env, stdin=subprocess.DEVNULL, capture_output=True,
                          text=True)
    check(binary, point, proc)
    rows = classify(trace)
    if point.startswith("archive/"):
        rows = [r for r in rows if r.startswith("open\t") and r.endswith("\t.jit/config.toml")]
    else:
        rows = [r for r in rows if not r.startswith("open\tfile\t")]
    name = f"r{round_index}-{binary}-{point.replace('/', '-')}.tsv"
    (raw / "traces" / name).write_text("".join(row + "\n" for row in rows))
    return name


print(f"[cost-bench] waiting up to {load_timeout}s for the 1-minute load below {load_max}",
      file=sys.stderr)
waited_from = time.monotonic()
while float(loadavg()[0]) >= load_max:
    if time.monotonic() - waited_from >= load_timeout:
        print(f"benchmark-cost-evidence: 1-minute load stayed at or above {load_max}: "
              f"{loadavg()}", file=sys.stderr)
        sys.exit(3)
    time.sleep(10)
load_waited = round(time.monotonic() - waited_from)
load_start = loadavg()

(raw / "traces").mkdir(parents=True)
timed_points = [f"bulk/N{n}-M{m}" for n, m in BULK] + [f"archive/C{c}" for c in ARCHIVE] + \
    ["dep/minimal", "dep/inflated"]
traced_points = [f"archive/C{c}" for c in ARCHIVE] + ["profile/package48"]
timings, traces = [], []
for round_index in range(samples):
    order = binaries if round_index % 2 == 0 else binaries[::-1]
    for binary in order:
        print(f"[cost-bench] round {round_index} {binary}", file=sys.stderr)
        for point in timed_points:
            timings.append({"round": round_index, "binary": binary, "point": point,
                            "wall_ns": timed(binary, point)})
        for point in traced_points:
            traces.append({"round": round_index, "binary": binary, "point": point,
                           "trace": traced(binary, point, round_index)})
load_end = loadavg()


def fs_type(path):
    out = subprocess.run(["df", "--output=fstype", str(path)], capture_output=True, text=True)
    return out.stdout.split()[-1]


host = {
    "label": label,
    "samples": samples,
    "harness_head": harness_head,
    "logical_cpus": os.cpu_count(),
    "cpu_model": next((line.split(": ", 1)[1].strip()
                       for line in Path("/proc/cpuinfo").read_text().splitlines()
                       if line.startswith("model name")), "unknown"),
    "kernel": f"{platform.system()} {platform.release()}",
    "strace": subprocess.run(["strace", "-V"], capture_output=True,
                             text=True).stdout.splitlines()[0],
    "measurement_fs": fs_type(work),
    "run_dir": str(run_dir),
    "load_wait": {"threshold": load_max, "timeout_s": load_timeout, "waited_s": load_waited},
    "loadavg_at_start": load_start,
    "loadavg_at_end": load_end,
    "binaries": {
        b: {
            "commit": base_sha if b == "base" else None,
            "sha256": hashlib.sha256(bins[b].read_bytes()).hexdigest(),
            "version": jit(b, work, "--version").stdout.strip(),
        }
        for b in binaries
    },
    "fixtures": fixtures,
    "argv": {point: argv_for(binaries[0], point) for point in timed_points + traced_points},
}
(raw / "host.json").write_text(json.dumps(host, indent=2) + "\n")
(raw / "timings.json").write_text(json.dumps(timings, indent=1) + "\n")
(raw / "traces.json").write_text(json.dumps(traces, indent=1) + "\n")
PYEOF

if [[ -n "${COST_BENCH_RAW:-}" ]]; then
  python3 -I "$EVIDENCE/validate.py" --raw "$RAW"
else
  python3 -I "$EVIDENCE/validate.py" --write
fi
