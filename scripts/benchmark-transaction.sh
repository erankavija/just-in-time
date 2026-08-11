#!/usr/bin/env bash
set -euo pipefail

# Warm transaction benchmark for the model-limit profile-package fixture.
#
# The operation/root mapping is deliberate:
#   - CLI `profile pack` and `profile add` require an initialized repository and
#     measure the existing-data-root journal shape;
#   - a temporary benchmark-only Rust helper calls those same public command
#     methods over an absent `.jit` layout, measuring the external-bootstrap
#     journal without weakening the CLI's repository precondition;
#   - CLI `init --profile` remains a supplemental absent-data-root publication,
#     not a substitute for the absent-root pack/add round trip.
#
# Each operation is measured normally and with directory fsync suppressed by a
# temporary LD_PRELOAD shim. The latter is NOT a safe implementation: it is an
# upper-bound probe for residual directory-durability cost. The recorded
# decision can recommend only deduplicating directory syncs behind an equivalent
# phase barrier; it never recommends removing a durability barrier.
#
# Every successful invocation appends one record to the stable JSON artifact.
# Existing records are compared before publication and are never changed.
#
# Environment overrides:
#   TRANSACTION_BENCH_JIT       jit binary (default: command -v jit)
#   TRANSACTION_BENCH_WARMUP    warmup runs per variant/scenario (default: 2)
#   TRANSACTION_BENCH_SAMPLES   measured runs per variant/scenario (default: 5)
#   TRANSACTION_BENCH_ARTIFACT  output artifact path
#   TRANSACTION_BENCH_PHASE     record phase, e.g. initial/rerun/post-change
#   TRANSACTION_BENCH_HELPER_TARGET  cached Cargo target dir for the helper
#
# Usage:
#   scripts/benchmark-transaction.sh
#   scripts/benchmark-transaction.sh --self-test

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
readonly SCRIPT_DIR
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
readonly REPO_ROOT
readonly DEFAULT_ARTIFACT="$REPO_ROOT/dev/benchmarks/test-suite-performance-4b7c06d0.json"

MODE="benchmark"
case "${1:-}" in
  "") ;;
  --self-test)
    MODE="self-test"
    shift
    ;;
  *)
    echo "usage: $0 [--self-test]" >&2
    exit 2
    ;;
esac
[ "$#" -eq 0 ] || {
  echo "usage: $0 [--self-test]" >&2
  exit 2
}

command -v python3 >/dev/null 2>&1 || {
  echo "transaction benchmark: 'python3' not found" >&2
  exit 2
}
command -v git >/dev/null 2>&1 || {
  echo "transaction benchmark: 'git' not found" >&2
  exit 2
}

if [ "$MODE" = "self-test" ]; then
  exec python3 - "$REPO_ROOT" <<'PYEOF'
import ast
import copy
import json
import pathlib
import re
import tempfile

repo = pathlib.Path(__import__("sys").argv[1])


def rust_usize_constant(source: str, name: str) -> int:
    match = re.search(rf"pub const {re.escape(name)}: usize = ([^;]+);", source)
    if not match:
        raise AssertionError(f"missing production authority {name}")
    tree = ast.parse(match.group(1), mode="eval")
    allowed = (ast.Expression, ast.Constant, ast.BinOp, ast.Add, ast.Sub,
               ast.Mult, ast.FloorDiv, ast.Div, ast.Mod)
    if any(not isinstance(node, allowed) for node in ast.walk(tree)):
        raise AssertionError(f"unsupported expression for {name}: {match.group(1)}")

    def evaluate(node):
        if isinstance(node, ast.Expression):
            return evaluate(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, int):
            return node.value
        left, right = evaluate(node.left), evaluate(node.right)
        if isinstance(node.op, ast.Add):
            return left + right
        if isinstance(node.op, ast.Sub):
            return left - right
        if isinstance(node.op, ast.Mult):
            return left * right
        if isinstance(node.op, (ast.Div, ast.FloorDiv)):
            return left // right
        if isinstance(node.op, ast.Mod):
            return left % right
        raise AssertionError("unreachable expression node")

    return evaluate(tree)


def stats(samples):
    ordered = sorted(samples)
    return {
        "samples_ms": samples,
        "median_ms": ordered[len(ordered) // 2],
        "min_ms": ordered[0],
        "max_ms": ordered[-1],
    }


def decision(measurements):
    pairs = {}
    for row in measurements:
        pairs.setdefault((row["operation"], row["root_shape"]), {})[
            row["variant"]
        ] = row
    savings = []
    for key, variants in pairs.items():
        baseline = variants["durable_baseline"]
        suppressed = variants["directory_fsync_suppressed"]
        paired = [
            durable - without_sync
            for durable, without_sync in zip(
                baseline["samples_ms"], suppressed["samples_ms"]
            )
        ]
        absolute = max(0, stats(paired)["median_ms"])
        savings.append((
            key, absolute,
            0 if baseline["median_ms"] == 0 else absolute / baseline["median_ms"],
        ))
    material = any(
        key[0] in ("profile_pack", "profile_add")
        and absolute >= 5
        and ratio >= 0.05
        for key, absolute, ratio in savings
    )
    return "deduplicate_safe_redundant_directory_syncs" if material else "no_change"


def assert_full_pack_add_matrix(measurements):
    required = {
        (operation, root_shape)
        for operation in ("profile_pack", "profile_add")
        for root_shape in ("existing_data_root", "absent_data_root")
    }
    observed = {(row["operation"], row["root_shape"]) for row in measurements}
    assert required <= observed, f"missing benchmark scenarios: {sorted(required - observed)}"


source = (repo / "crates/jit/src/profile/package.rs").read_text()
assert rust_usize_constant(source, "MAX_PROFILE_PACKAGE_FILES") > 1
assert rust_usize_constant(source, "MAX_PROFILE_PACKAGE_BYTES") > 1024
assert stats([9, 1, 4])["median_ms"] == 4

synthetic = [
    {"operation": "profile_add", "root_shape": "existing_data_root",
     "variant": "durable_baseline", "median_ms": 120,
     "samples_ms": [118, 120, 122]},
    {"operation": "profile_add", "root_shape": "existing_data_root",
     "variant": "directory_fsync_suppressed", "median_ms": 100,
     "samples_ms": [100, 100, 101]},
]
assert decision(synthetic) == "deduplicate_safe_redundant_directory_syncs"
synthetic[1].update({"median_ms": 119, "samples_ms": [118, 119, 120]})
assert decision(synthetic) == "no_change"
synthetic.extend([
    {"operation": "profiled_init_publication", "root_shape": "absent_data_root",
     "variant": "durable_baseline", "median_ms": 200,
     "samples_ms": [198, 200, 202]},
    {"operation": "profiled_init_publication", "root_shape": "absent_data_root",
     "variant": "directory_fsync_suppressed", "median_ms": 100,
     "samples_ms": [100, 100, 101]},
])
assert decision(synthetic) == "no_change", "supplemental init changed the pack/add decision"
matrix = [
    {"operation": operation, "root_shape": root_shape}
    for operation in ("profile_pack", "profile_add")
    for root_shape in ("existing_data_root", "absent_data_root")
]
assert_full_pack_add_matrix(matrix)
try:
    assert_full_pack_add_matrix(matrix[:-1])
except AssertionError:
    pass
else:
    raise AssertionError("an incomplete operation/root-shape matrix was accepted")

with tempfile.TemporaryDirectory() as raw:
    artifact = pathlib.Path(raw) / "evidence.json"
    document = {"schema_version": 1, "records": [{"sequence": 1, "value": "first"}]}
    artifact.write_text(json.dumps(document))
    original = copy.deepcopy(document["records"])
    loaded = json.loads(artifact.read_text())
    loaded["records"].append({"sequence": 2, "value": "second"})
    artifact.write_text(json.dumps(loaded))
    observed = json.loads(artifact.read_text())
    assert observed["records"][:-1] == original
    assert observed["records"][-1]["sequence"] == 2

print("transaction benchmark self-test: PASS")
PYEOF
fi

for tool in cc cargo; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "transaction benchmark: '$tool' not found" >&2
    exit 2
  }
done

JIT_BIN=${TRANSACTION_BENCH_JIT:-$(command -v jit || true)}
[ -n "$JIT_BIN" ] && [ -x "$JIT_BIN" ] || {
  echo "transaction benchmark: set TRANSACTION_BENCH_JIT to an executable jit binary" >&2
  exit 2
}
JIT_BIN=$(cd "$(dirname "$JIT_BIN")" && pwd)/$(basename "$JIT_BIN")

WARMUP=${TRANSACTION_BENCH_WARMUP:-2}
SAMPLES=${TRANSACTION_BENCH_SAMPLES:-5}
ARTIFACT=${TRANSACTION_BENCH_ARTIFACT:-$DEFAULT_ARTIFACT}
PHASE=${TRANSACTION_BENCH_PHASE:-rerun}

case "$WARMUP" in
  ''|*[!0-9]*) echo "transaction benchmark: warmup must be an integer" >&2; exit 2 ;;
esac
case "$SAMPLES" in
  ''|*[!0-9]*) echo "transaction benchmark: samples must be an integer" >&2; exit 2 ;;
esac
[ "$WARMUP" -ge 1 ] || {
  echo "transaction benchmark: warmup must be at least 1" >&2
  exit 2
}
[ "$SAMPLES" -ge 3 ] || {
  echo "transaction benchmark: samples must be at least 3" >&2
  exit 2
}
case "$PHASE" in
  initial|rerun|post-change) ;;
  *)
    echo "transaction benchmark: phase must be initial, rerun, or post-change" >&2
    exit 2
    ;;
esac

scratch=$(mktemp -d)
cleanup() { rm -rf "$scratch"; }
trap cleanup EXIT

# The comparator preserves file fsync and suppresses only directory fsync.
# Both variants load the shim, so its fstat/getenv overhead cancels out.
cat >"$scratch/directory-fsync-shim.c" <<'CEOF'
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

typedef int (*sync_fn)(int);

static int call_sync(const char *symbol, int fd) {
    static sync_fn real_fsync = NULL;
    static sync_fn real_fdatasync = NULL;
    sync_fn *slot = symbol[1] == 's' ? &real_fsync : &real_fdatasync;
    if (*slot == NULL) {
        *slot = (sync_fn)dlsym(RTLD_NEXT, symbol);
        if (*slot == NULL) {
            errno = ENOSYS;
            return -1;
        }
    }
    struct stat metadata;
    if (fstat(fd, &metadata) == 0 && S_ISDIR(metadata.st_mode)) {
        const char *log_path = getenv("JIT_BENCH_DIRECTORY_FSYNC_LOG");
        if (log_path != NULL) {
            int log_fd = open(log_path, O_WRONLY | O_CREAT | O_APPEND, 0600);
            if (log_fd >= 0) {
                (void)write(log_fd, "1\n", 2);
                (void)close(log_fd);
            }
        }
        if (getenv("JIT_BENCH_SKIP_DIRECTORY_FSYNC") != NULL) {
            return 0;
        }
    }
    return (*slot)(fd);
}

int fsync(int fd) { return call_sync("fsync", fd); }
int fdatasync(int fd) { return call_sync("fdatasync", fd); }
CEOF
cc -shared -fPIC -O2 -Wall -Wextra -Werror \
  -o "$scratch/directory-fsync-shim.so" "$scratch/directory-fsync-shim.c" -ldl

# Pack/add are CLI-invalid before repository initialization, but their public
# command-layer methods deliberately support worktree-only transactions over an
# absent data root. Compile a temporary driver once, outside every timed region,
# to exercise that exact external-bootstrap journal path without production
# code or a benchmark-only product switch.
HELPER_TARGET=${TRANSACTION_BENCH_HELPER_TARGET:-$REPO_ROOT/target/transaction-benchmark-helper}
mkdir -p "$scratch/helper/src"
cat >"$scratch/helper/Cargo.toml" <<EOF
[package]
name = "transaction-benchmark-helper"
version = "0.0.0"
edition = "2021"

[dependencies]
jit = { path = "$REPO_ROOT/crates/jit" }
EOF
cat >"$scratch/helper/src/main.rs" <<'RSEOF'
use jit::storage::discover_repository_layout;
use jit::{CommandExecutor, JsonFileStorage};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

fn count_files(root: &Path) -> std::io::Result<usize> {
    fs::read_dir(root)?.try_fold(0, |count, entry| {
        let path = entry?.path();
        Ok(count
            + if path.is_dir() {
                count_files(&path)?
            } else {
                usize::from(path.is_file())
            })
    })
}

fn ensure_absent_root(worktree: &Path, moment: &str) -> Result<(), Box<dyn Error>> {
    if worktree.join(".jit").exists() {
        return Err(std::io::Error::other(format!(
            "data root exists {moment}: {}",
            worktree.join(".jit").display()
        ))
        .into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments.len() != 5 {
        return Err(std::io::Error::other(
            "usage: transaction-benchmark-helper <pack|add> <worktree> <input> <output>",
        )
        .into());
    }
    let operation = &arguments[1];
    let worktree = PathBuf::from(&arguments[2]);
    let input = PathBuf::from(&arguments[3]);
    let output = PathBuf::from(&arguments[4]);
    let data_root = worktree.join(".jit");
    ensure_absent_root(&worktree, "before command")?;
    let layout = discover_repository_layout(&worktree, &data_root)?;
    let executor = CommandExecutor::new(JsonFileStorage::new(&data_root)).with_layout(layout);

    let file_count = match operation.as_str() {
        "pack" => {
            let (result, _) = executor.pack_profile_package(&worktree, &input, &output)?;
            if !worktree.join(&output).is_file() {
                return Err(std::io::Error::other("pack did not publish its archive").into());
            }
            result.file_count
        }
        "add" => {
            let result = executor.add_profile_package(&worktree, &input, &output)?;
            let published_count = count_files(&worktree.join(&output))?;
            if published_count != result.file_count {
                return Err(std::io::Error::other(format!(
                    "add reported {} files but published {published_count}",
                    result.file_count
                ))
                .into());
            }
            result.file_count
        }
        _ => return Err(std::io::Error::other("operation must be pack or add").into()),
    };
    ensure_absent_root(&worktree, "after command")?;
    if worktree.join(".jit-bootstrap").exists() {
        return Err(std::io::Error::other("successful command left bootstrap residue").into());
    }
    println!("file_count={file_count}");
    Ok(())
}
RSEOF
cp "$REPO_ROOT/Cargo.lock" "$scratch/helper/Cargo.lock"
CARGO_INCREMENTAL=0 cargo build --quiet --release --offline \
  --manifest-path "$scratch/helper/Cargo.toml" --target-dir "$HELPER_TARGET"
HELPER_BIN="$HELPER_TARGET/release/transaction-benchmark-helper"
[ -x "$HELPER_BIN" ] || {
  echo "transaction benchmark: helper build did not produce $HELPER_BIN" >&2
  exit 2
}

mkdir -p "$(dirname "$ARTIFACT")"

exec python3 - "$REPO_ROOT" "$JIT_BIN" "$WARMUP" "$SAMPLES" "$ARTIFACT" \
  "$PHASE" "$scratch" "$scratch/directory-fsync-shim.so" "$HELPER_BIN" <<'PYEOF'
import ast
import copy
import datetime as dt
import fcntl
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

repo = pathlib.Path(sys.argv[1])
jit = pathlib.Path(sys.argv[2])
warmup = int(sys.argv[3])
sample_count = int(sys.argv[4])
artifact = pathlib.Path(sys.argv[5])
phase = sys.argv[6]
scratch = pathlib.Path(sys.argv[7])
shim = pathlib.Path(sys.argv[8])
helper = pathlib.Path(sys.argv[9])


def run(argv, cwd, env=None):
    completed = subprocess.run(
        [str(part) for part in argv], cwd=cwd, env=env,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(map(str, argv))}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )
    return completed


def json_output(completed):
    decoded = json.loads(completed.stdout)
    return decoded.get("data", decoded)


def rust_usize_constant(source, name):
    match = re.search(rf"pub const {re.escape(name)}: usize = ([^;]+);", source)
    if not match:
        raise RuntimeError(f"production authority does not declare {name}")
    tree = ast.parse(match.group(1), mode="eval")
    allowed = (ast.Expression, ast.Constant, ast.BinOp, ast.Add, ast.Sub,
               ast.Mult, ast.FloorDiv, ast.Div, ast.Mod)
    if any(not isinstance(node, allowed) for node in ast.walk(tree)):
        raise RuntimeError(f"unsupported expression for {name}: {match.group(1)}")

    def evaluate(node):
        if isinstance(node, ast.Expression):
            return evaluate(node.body)
        if isinstance(node, ast.Constant) and isinstance(node.value, int):
            return node.value
        left, right = evaluate(node.left), evaluate(node.right)
        if isinstance(node.op, ast.Add):
            return left + right
        if isinstance(node.op, ast.Sub):
            return left - right
        if isinstance(node.op, ast.Mult):
            return left * right
        if isinstance(node.op, (ast.Div, ast.FloorDiv)):
            return left // right
        if isinstance(node.op, ast.Mod):
            return left % right
        raise RuntimeError("unsupported expression node")

    return evaluate(tree)


def write_model_limit_package(root, max_files, max_bytes):
    assets = max_files - 1
    padding = "x" * 70

    def source(index):
        return (
            f"assets/outer-{padding}-{index:03}/"
            f"inner-{padding}-{index:03}/source-{padding}-{index:03}.txt"
        )

    manifest = (
        '[profile]\nmanifest-version = 1\nid = "bounded-package"\n'
        'version = "1.0.0"\njit = ">=0.2.0, <2.0.0"\n'
    )
    manifest += "".join(
        f'\n[[asset]]\nsource = "{source(index)}"\n'
        f'target = "out/target-{padding}-{index:03}.txt"\n'
        for index in range(assets)
    )
    filler = max(0, max_bytes - len(manifest.encode())) // assets
    root.mkdir(parents=True)
    (root / "manifest.toml").write_text(manifest)
    for index in range(assets):
        path = root / source(index)
        path.parent.mkdir(parents=True)
        path.write_bytes(b"." * filler)

    files = [path for path in root.rglob("*") if path.is_file()]
    directories = [path for path in root.rglob("*") if path.is_dir()]
    total_bytes = sum(path.stat().st_size for path in files)
    return {
        "file_count": len(files),
        "byte_size": total_bytes,
        "max_source_path_bytes": max(
            len(path.relative_to(root).as_posix().encode()) for path in files
        ),
        "distinct_source_directories": len(directories),
        "asset_count": assets,
    }


def initialized_repo(path):
    path.mkdir(parents=True)
    run([jit, "init", "--json"], path)


def timed_variants(operation, root_shape, execution_boundary, setup, invoke, verify, cleanup):
    variants = ("durable_baseline", "directory_fsync_suppressed")

    def variant_env(variant):
        env = os.environ.copy()
        env["LD_PRELOAD"] = str(shim)
        if variant == "directory_fsync_suppressed":
            env["JIT_BENCH_SKIP_DIRECTORY_FSYNC"] = "1"
        else:
            env.pop("JIT_BENCH_SKIP_DIRECTORY_FSYNC", None)
        return env

    def once(variant):
        env = variant_env(variant)
        context = setup()
        start = time.perf_counter_ns()
        completed = run(invoke(context), context["cwd"], env)
        elapsed_ms = (time.perf_counter_ns() - start + 500_000) // 1_000_000
        verify(context, completed)
        cleanup(context)
        return int(elapsed_ms)

    # Alternate the order within each pair. This keeps progressive cache or
    # host-load changes from always favoring the same comparator variant.
    for index in range(warmup):
        order = variants if index % 2 == 0 else tuple(reversed(variants))
        for variant in order:
            once(variant)
    samples = {variant: [] for variant in variants}
    for index in range(sample_count):
        order = variants if index % 2 == 0 else tuple(reversed(variants))
        observed = {variant: once(variant) for variant in order}
        for variant in variants:
            samples[variant].append(observed[variant])

    def row(variant):
        values = samples[variant]
        ordered = sorted(values)
        return {
            "operation": operation,
            "root_shape": root_shape,
            "execution_boundary": execution_boundary,
            "variant": variant,
            "unit": "integer_ms",
            "samples_ms": values,
            "median_ms": int(statistics.median(values)),
            "min_ms": ordered[0],
            "max_ms": ordered[-1],
        }

    return [row(variant) for variant in variants]


def count_directory_fsyncs(
        operation, root_shape, execution_boundary, setup, invoke, verify, cleanup):
    log = scratch / f"directory-fsync-{operation}-{root_shape}.log"
    log.unlink(missing_ok=True)
    env = os.environ.copy()
    env["LD_PRELOAD"] = str(shim)
    env["JIT_BENCH_DIRECTORY_FSYNC_LOG"] = str(log)
    env.pop("JIT_BENCH_SKIP_DIRECTORY_FSYNC", None)
    context = setup()
    completed = run(invoke(context), context["cwd"], env)
    verify(context, completed)
    cleanup(context)
    calls = 0 if not log.exists() else len(log.read_text().splitlines())
    if calls == 0:
        raise RuntimeError(f"directory-fsync comparator intercepted no calls for {operation}")
    return {
        "operation": operation,
        "root_shape": root_shape,
        "execution_boundary": execution_boundary,
        "directory_fsync_calls": calls,
        "timed": False,
    }


package_source = (repo / "crates/jit/src/profile/package.rs").read_text()
max_files = rust_usize_constant(package_source, "MAX_PROFILE_PACKAGE_FILES")
max_bytes = rust_usize_constant(package_source, "MAX_PROFILE_PACKAGE_BYTES")

fixtures = scratch / "fixtures"
source_repo = fixtures / "source-existing"
target_repo = fixtures / "target-existing"
absent_template = fixtures / "absent-template"
initialized_repo(source_repo)
initialized_repo(target_repo)
absent_template.mkdir(parents=True)

source_package = source_repo / "packages/bounded"
fixture = write_model_limit_package(source_package, max_files, max_bytes)
shutil.copytree(source_package, absent_template / "packages/bounded")
absent_command_repo = fixtures / "command-absent"
shutil.copytree(source_package, absent_command_repo / "packages/bounded")
(absent_command_repo / "exchange").mkdir()

archive = source_repo / "exchange/bounded.tar"
archive.parent.mkdir(parents=True)
packed = json_output(run([
    jit, "profile", "pack", "--source", "packages/bounded",
    "--output", "exchange/bounded.tar", "--json",
], source_repo))
if packed["file_count"] != max_files:
    raise RuntimeError("profile pack did not observe the production file-count limit")
if packed["byte_size"] != fixture["byte_size"]:
    raise RuntimeError("profile pack observed different fixture bytes")
add_archive = fixtures / "bounded-for-add.tar"
shutil.copy2(archive, add_archive)
fixture.update({
    "package_hash": packed["package_hash"],
    "archive_bytes": packed["archive_bytes"],
})


def pack_setup():
    archive.unlink(missing_ok=True)
    return {"cwd": source_repo, "output": archive}


def pack_invoke(_):
    return [jit, "profile", "pack", "--source", "packages/bounded",
            "--output", "exchange/bounded.tar", "--json"]


def pack_verify(context, completed):
    payload = json_output(completed)
    if payload["file_count"] != max_files or not context["output"].is_file():
        raise RuntimeError("profile pack did not carry the complete model-limit fixture")


def pack_cleanup(context):
    context["output"].unlink()


def add_setup():
    destination = target_repo / "packages/arrived"
    shutil.rmtree(destination, ignore_errors=True)
    return {"cwd": target_repo, "destination": destination}


def add_invoke(_):
    return [jit, "profile", "add", "--archive", add_archive,
            "--destination", "packages/arrived", "--json"]


def add_verify(context, completed):
    payload = json_output(completed)
    count = sum(path.is_file() for path in context["destination"].rglob("*"))
    if payload["file_count"] != max_files or count != max_files:
        raise RuntimeError("profile add did not publish the complete model-limit fixture")


def add_cleanup(context):
    shutil.rmtree(context["destination"])


absent_archive = absent_command_repo / "exchange/bounded.tar"


def assert_absent_command_root():
    if (absent_command_repo / ".jit").exists():
        raise RuntimeError("absent command-layer fixture unexpectedly contains .jit")
    if (absent_command_repo / ".jit-bootstrap").exists():
        raise RuntimeError("absent command-layer fixture contains bootstrap residue")


def absent_pack_setup():
    absent_archive.unlink(missing_ok=True)
    assert_absent_command_root()
    return {"cwd": absent_command_repo, "output": absent_archive}


def absent_pack_invoke(_):
    return [helper, "pack", absent_command_repo, "packages/bounded", "exchange/bounded.tar"]


def absent_pack_verify(context, completed):
    if completed.stdout.strip() != f"file_count={max_files}" or not context["output"].is_file():
        raise RuntimeError("command-layer pack did not carry the complete model-limit fixture")
    assert_absent_command_root()


def absent_pack_cleanup(context):
    context["output"].unlink()
    assert_absent_command_root()


def absent_add_setup():
    destination = absent_command_repo / "packages/arrived"
    shutil.rmtree(destination, ignore_errors=True)
    assert_absent_command_root()
    return {"cwd": absent_command_repo, "destination": destination}


def absent_add_invoke(_):
    return [helper, "add", absent_command_repo, add_archive, "packages/arrived"]


def absent_add_verify(context, completed):
    count = sum(path.is_file() for path in context["destination"].rglob("*"))
    if completed.stdout.strip() != f"file_count={max_files}" or count != max_files:
        raise RuntimeError("command-layer add did not publish the complete model-limit fixture")
    assert_absent_command_root()


def absent_add_cleanup(context):
    shutil.rmtree(context["destination"])
    assert_absent_command_root()


init_counter = 0


def init_setup():
    global init_counter
    init_counter += 1
    destination = scratch / "absent-runs" / f"run-{init_counter}"
    shutil.copytree(absent_template, destination)
    if (destination / ".jit").exists():
        raise RuntimeError("absent-root fixture unexpectedly contains .jit")
    return {"cwd": destination, "destination": destination}


def init_invoke(_):
    return [jit, "init", "--profile", "path:packages/bounded", "--json"]


def init_verify(context, completed):
    json_output(completed)
    output_count = sum(path.is_file() for path in (context["destination"] / "out").rglob("*"))
    if not (context["destination"] / ".jit").is_dir() or output_count != fixture["asset_count"]:
        raise RuntimeError("profiled init did not publish the absent data root and every asset")


def init_cleanup(context):
    shutil.rmtree(context["destination"])


measurements = []
directory_fsync_probes = []
scenarios = [
    ("profile_pack", "existing_data_root", "cli", pack_setup, pack_invoke,
     pack_verify, pack_cleanup),
    ("profile_add", "existing_data_root", "cli", add_setup, add_invoke,
     add_verify, add_cleanup),
    ("profile_pack", "absent_data_root", "public_command_layer_helper", absent_pack_setup,
     absent_pack_invoke, absent_pack_verify, absent_pack_cleanup),
    ("profile_add", "absent_data_root", "public_command_layer_helper", absent_add_setup,
     absent_add_invoke, absent_add_verify, absent_add_cleanup),
    ("profiled_init_publication", "absent_data_root", "cli", init_setup, init_invoke,
     init_verify, init_cleanup),
]
for operation, root_shape, execution_boundary, setup, invoke, verify, cleanup in scenarios:
    print(f"[transaction-bench] {operation}/{root_shape}/paired-variants", file=sys.stderr)
    measurements.extend(timed_variants(
        operation, root_shape, execution_boundary, setup, invoke, verify, cleanup
    ))
    directory_fsync_probes.append(count_directory_fsyncs(
        operation, root_shape, execution_boundary, setup, invoke, verify, cleanup
    ))

required_pack_add_matrix = {
    (operation, root_shape)
    for operation in ("profile_pack", "profile_add")
    for root_shape in ("existing_data_root", "absent_data_root")
}
observed_pack_add_matrix = {
    (row["operation"], row["root_shape"]) for row in measurements
}
missing_pack_add_scenarios = required_pack_add_matrix - observed_pack_add_matrix
if missing_pack_add_scenarios:
    raise RuntimeError(f"incomplete pack/add root-shape matrix: {sorted(missing_pack_add_scenarios)}")


def fsync_decision(rows, probes):
    grouped = {}
    for row in rows:
        grouped.setdefault((row["operation"], row["root_shape"]), {})[
            row["variant"]
        ] = row["median_ms"]
    savings = []
    for (operation, root_shape), variants in grouped.items():
        baseline = variants["durable_baseline"]
        suppressed = variants["directory_fsync_suppressed"]
        baseline_row = next(
            row for row in rows
            if row["operation"] == operation and row["root_shape"] == root_shape
            and row["variant"] == "durable_baseline"
        )
        suppressed_row = next(
            row for row in rows
            if row["operation"] == operation and row["root_shape"] == root_shape
            and row["variant"] == "directory_fsync_suppressed"
        )
        paired = [
            durable - without_directory_sync
            for durable, without_directory_sync in zip(
                baseline_row["samples_ms"], suppressed_row["samples_ms"]
            )
        ]
        absolute = max(0, int(statistics.median(paired)))
        ratio = 0 if baseline == 0 else absolute / baseline
        savings.append({
            "operation": operation,
            "root_shape": root_shape,
            "decision_eligible": operation in ("profile_pack", "profile_add"),
            "baseline_median_ms": baseline,
            "suppressed_median_ms": suppressed,
            "paired_savings_ms": paired,
            "paired_median_saving_ms": absolute,
            "saving_percent": int(round(ratio * 100)),
        })
    material = any(
        row["decision_eligible"]
        and row["paired_median_saving_ms"] >= 5
        and row["saving_percent"] >= 5
        for row in savings
    ) and all(probe["directory_fsync_calls"] > 0 for probe in probes)
    return {
        "decision": (
            "deduplicate_safe_redundant_directory_syncs"
            if material else "no_change"
        ),
        "criteria": {
            "minimum_paired_median_saving_ms": 5,
            "minimum_paired_median_saving_percent": 5,
            "required_observation": (
                "both thresholds in at least one profile pack/add operation/root shape"
            ),
        },
        "decision_scope": (
            "profile pack/add across existing and absent data roots; profiled init is "
            "supplemental attribution evidence and cannot change the decision"
        ),
        "observations": savings,
        "constraint": (
            "The suppressed variant is an unsafe upper bound, not a candidate implementation. "
            "A production change may only deduplicate repeated directory syncs behind the "
            "existing preparation or pre-commit barrier; file fsyncs and phase barriers remain."
        ),
    }


version = json_output(run([jit, "version", "--json"], repo))
version["sha256"] = hashlib.sha256(jit.read_bytes()).hexdigest()
source_revision = run(["git", "rev-parse", "HEAD"], repo).stdout.strip()
dirty = subprocess.run(["git", "diff", "--quiet"], cwd=repo).returncode != 0
filesystem = run(["stat", "-f", "-c", "%T", str(scratch)], repo).stdout.strip()
decision_result = fsync_decision(measurements, directory_fsync_probes)

record = {
    "sequence": 0,
    "record_schema_revision": 3,
    "recorded_at": dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat(),
    "phase": phase,
    "source": {"revision": source_revision, "dirty": dirty},
    "binary": version,
    "host": {
        "system": platform.system(),
        "release": platform.release(),
        "machine": platform.machine(),
        "measurement_filesystem": filesystem,
    },
    "method": {
        "clock": "Python time.perf_counter_ns around one jit subprocess",
        "warmup_runs_per_variant": warmup,
        "measured_runs_per_variant": sample_count,
        "cache_state": (
            "warm: binary and fixture pages primed by warmups; absent-root copies are created "
            "before timing; kernel page cache is not dropped"
        ),
        "cleanup": (
            "archive, add destination, and profiled-init repository are removed after each "
            "timed subprocess; cleanup is outside the clock"
        ),
        "root_shape_mapping": {
            "profile_pack_cli": "existing_data_root",
            "profile_add_cli": "existing_data_root",
            "profile_pack_command_layer": "absent_data_root",
            "profile_add_command_layer": "absent_data_root",
            "profiled_init_publication": "absent_data_root",
        },
        "absent_root_helper": {
            "build": (
                "workspace Cargo.lock plus CARGO_INCREMENTAL=0 cargo build --release --offline "
                "outside timed regions"
            ),
            "source": "temporary driver over public CommandExecutor profile pack/add methods",
            "sha256": hashlib.sha256(helper.read_bytes()).hexdigest(),
            "postcondition": ".jit and .jit-bootstrap remain absent after each command",
        },
        "fsync_comparator": (
            "LD_PRELOAD shim preserves file fsync and suppresses directory fsync only; both "
            "variants load the shim so interception overhead is shared; one separate untimed "
            "baseline probe per operation verifies and counts intercepted directory fsync calls"
        ),
        "comparison_order": (
            "durable and suppressed observations are paired; first-run order alternates "
            "between pairs to limit progressive cache and host-load bias"
        ),
    },
    "fixture": {
        "authority": "crates/jit/src/profile/package.rs",
        "shape_authority": "crates/jit/src/test_utils.rs::write_package_tree_at_model_limits",
        "file_limit": max_files,
        "byte_limit": max_bytes,
        **fixture,
    },
    "measurements": measurements,
    "directory_fsync_probes": directory_fsync_probes,
    "fsync_residual_decision": decision_result,
}
if phase == "post-change":
    record["applied_fsync_decision"] = {
        "decision": decision_result["decision"],
        "criteria": decision_result["criteria"],
        "scope": decision_result["decision_scope"],
        "outcome": (
            "no production fsync change warranted by this record"
            if decision_result["decision"] == "no_change"
            else "material residual remains; this record asserts no production application"
        ),
    }

contract = {
    "schema_version": 1,
    "contract": "transaction-benchmark-evidence",
    "append_policy": (
        "records are chronological and immutable; producers append one complete record and "
        "never rewrite an earlier record"
    ),
    "records": [],
}
artifact_key = hashlib.sha256(str(artifact.resolve()).encode()).hexdigest()[:16]
lock_path = pathlib.Path(tempfile.gettempdir()) / f"jit-transaction-benchmark-{artifact_key}.lock"
with lock_path.open("a+") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    if artifact.exists():
        contract = json.loads(artifact.read_text())
        if (contract.get("schema_version") != 1 or
                contract.get("contract") != "transaction-benchmark-evidence"):
            raise RuntimeError(f"refusing incompatible artifact {artifact}")
    prior = copy.deepcopy(contract["records"])
    record["sequence"] = len(prior) + 1
    if not prior:
        record["phase"] = "initial"
    contract["records"].append(record)
    if contract["records"][:-1] != prior:
        raise RuntimeError("append changed an earlier benchmark record")

    artifact.parent.mkdir(parents=True, exist_ok=True)
    temporary = artifact.with_name(f".{artifact.name}.{os.getpid()}.tmp")
    temporary.write_text(json.dumps(contract, indent=2, sort_keys=True) + "\n")
    json.loads(temporary.read_text())
    os.replace(temporary, artifact)

print(json.dumps({
    "artifact": str(artifact),
    "sequence": record["sequence"],
    "phase": record["phase"],
    "fsync_residual_decision": record["fsync_residual_decision"]["decision"],
    "measurements": measurements,
}, indent=2))
PYEOF
