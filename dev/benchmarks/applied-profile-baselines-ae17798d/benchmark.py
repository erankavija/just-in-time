#!/usr/bin/env python3
"""Capture exact setup-inclusive evidence for ae17798d REQ-04."""

from __future__ import annotations

import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import subprocess
import time


ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = Path(__file__).resolve().parent
RAW = EVIDENCE / "raw"
LOCK = Path("/tmp/cargo-ci.lock")
TARGET = Path("/home/vkaskivuo/Projects/just-in-time/target")
SOURCE_REVISION = "db82ac6cc5344d8d8343e76f058e71f8c55812c4"
IMPLEMENTATION_COMMIT = "61234ddf0962536303e0447e0a8e530d2473a9c6"
REFERENCE_MS = 64_894
MAXIMUM_SPAN_MS = 25_000
MINIMUM_REDUCTION_MS = 25_000
SETUP_PATTERN = re.compile(r"SETUP PASS \[\s*([0-9.]+)s\] ([^:]+):")
TEST_NAMES = (
    "commands::init::tests::test_single_profile_reinit_attributes_coupled_schema_repair",
    "commands::profile::tests::test_profile_preparation_retries_when_final_proposed_closure_expands",
    "commands::init::tests::test_reinit_profiled_over_existing_root_is_idempotent_unchanged",
    "commands::profile::tests::test_apply_profile_selection_recovers_an_obsolete_default_schema_atomically",
    "commands::validate::tests::test_validate_fix_repairs_every_owned_materialization_and_preserves_unowned_files",
    "commands::validate::tests::test_validate_fix_rejects_ambiguous_region_without_writing_other_repairs",
    "commands::validate::tests::test_validate_fix_repairs_owned_materializations_in_memory",
    "commands::validate::tests::test_validate_fix_rejects_ambiguous_region_without_memory_writes",
    "commands::validate::tests::test_capture_repair_plan_with_recorded_profile_captures_and_repairs_its_targets",
    "commands::validate::tests::test_validate_diagnoses_non_profile_drift_identically_across_record_states",
)
IDENTITIES = tuple(f"jit::jit${name}" for name in TEST_NAMES)
FILTER = "binary(jit) & test(/^({})$/)".format("|".join(TEST_NAMES))
RUN_COMMAND = (
    "cargo",
    "nextest",
    "run",
    "--workspace",
    "--test-threads",
    "24",
    "--message-format",
    "libtest-json-plus",
    "--message-format-version",
    "0.1",
    "--show-progress",
    "none",
    "--status-level",
    "pass",
    "--final-status-level",
    "none",
    "-E",
    FILTER,
)
INVENTORY_COMMAND = (
    "cargo",
    "nextest",
    "list",
    "--workspace",
    "--message-format",
    "json",
    "-E",
    FILTER,
)
CANDIDATE_PATHS = (
    ".config/nextest.toml",
    "crates/jit/src/commands/init.rs",
    "crates/jit/src/commands/profile.rs",
    "crates/jit/src/commands/validate.rs",
    "crates/jit/src/storage/test_support.rs",
    "crates/jit/src/test_utils.rs",
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def command_output(command: tuple[str, ...] | list[str]) -> str:
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def selected_inventory(raw: bytes) -> list[str]:
    document = json.loads(raw)
    identities = []
    for binary, suite in document["rust-suites"].items():
        for test_name, test in suite["testcases"].items():
            if test["filter-match"]["status"] == "matches":
                identities.append(f"{suite['package-name']}::{binary}${test_name}")
    return sorted(identities)


def test_events(raw: bytes) -> list[dict[str, object]]:
    events = []
    for line in raw.decode("utf-8").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "test":
            events.append(event)
    return events


def run(label: str, command: tuple[str, ...], environment: dict[str, str]) -> dict[str, object]:
    started_ns = time.monotonic_ns()
    process = subprocess.run(
        command,
        cwd=ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    ended_ns = time.monotonic_ns()
    stdout_path = RAW / f"{label}.stdout"
    stderr_path = RAW / f"{label}.stderr"
    stdout_path.write_bytes(process.stdout)
    stderr_path.write_bytes(process.stderr)
    if process.returncode != 0:
        raise SystemExit(f"{label}: exit {process.returncode}; see {stderr_path}")

    events = test_events(process.stdout)
    started = [str(event["name"]) for event in events if event.get("event") == "started"]
    results = [
        event
        for event in events
        if event.get("event") in ("ok", "failed", "ignored")
    ]
    passed = [event for event in results if event.get("event") == "ok"]
    if sorted(started) != sorted(IDENTITIES) or sorted(
        str(event["name"]) for event in results
    ) != sorted(IDENTITIES):
        raise SystemExit(f"{label}: exact identity control failed")
    if len(set(started)) != len(started) or len(
        {str(event["name"]) for event in results}
    ) != len(results):
        raise SystemExit(f"{label}: duplicate test event")
    if len(passed) != len(IDENTITIES):
        raise SystemExit(f"{label}: not all selected tests passed")

    setup_matches = SETUP_PATTERN.findall(process.stderr.decode("utf-8"))
    setups = {name: round(float(seconds) * 1_000) for seconds, name in setup_matches}
    if len(setup_matches) != 2 or set(setups) != {
        "default-repository-fixture",
        "dogfood-repository-fixture",
    }:
        raise SystemExit(f"{label}: setup control failed: {setup_matches}")

    tests = {
        str(event["name"]): math.ceil(float(event["exec_time"]) * 1_000)
        for event in passed
    }
    test_sum_ms = sum(tests.values())
    setup_sum_ms = sum(setups.values())
    span_ms = test_sum_ms + setup_sum_ms
    return {
        "label": label,
        "exit_code": process.returncode,
        "stdout": str(stdout_path.relative_to(EVIDENCE)),
        "stderr": str(stderr_path.relative_to(EVIDENCE)),
        "wall_ms": round((ended_ns - started_ns) / 1_000_000),
        "setups_ms": setups,
        "setup_sum_ms": setup_sum_ms,
        "tests_ms": dict(sorted(tests.items())),
        "test_sum_ms": test_sum_ms,
        "setup_inclusive_span_ms": span_ms,
        "reduction_from_reference_ms": REFERENCE_MS - span_ms,
    }


if RAW.exists() and any(RAW.iterdir()):
    raise SystemExit(f"refusing to overwrite raw evidence in {RAW}")
RAW.mkdir(parents=True, exist_ok=True)

revision = command_output(("git", "rev-parse", "HEAD"))
if revision != SOURCE_REVISION:
    raise SystemExit(f"expected {SOURCE_REVISION}, found {revision}")
dirty_paths = command_output(("git", "status", "--porcelain", "--untracked-files=all"))
unexpected_dirty = [
    line
    for line in dirty_paths.splitlines()
    if "dev/benchmarks/applied-profile-baselines-ae17798d/" not in line
]
if unexpected_dirty:
    raise SystemExit(f"unexpected dirty paths: {unexpected_dirty}")

environment = os.environ.copy()
environment.update(
    {
        "CARGO_TARGET_DIR": str(TARGET),
        "CARGO_INCREMENTAL": "0",
        "CARGO_CI_NO_SCCACHE": "1",
        "RUSTC_WRAPPER": "",
        "NEXTEST_EXPERIMENTAL_LIBTEST_JSON": "1",
    }
)
metadata = {
    "source_revision": revision,
    "source_tree": command_output(("git", "show", "-s", "--format=%T", revision)),
    "implementation_commit": IMPLEMENTATION_COMMIT,
    "host": {
        "hostname": platform.node(),
        "platform": platform.platform(),
        "logical_cpus": os.cpu_count(),
    },
    "tools": {
        "cargo": command_output(("cargo", "--version")),
        "rustc": command_output(("rustc", "-Vv")),
        "nextest": command_output(("cargo", "nextest", "--version")),
    },
    "environment": {
        key: environment[key]
        for key in (
            "CARGO_TARGET_DIR",
            "CARGO_INCREMENTAL",
            "CARGO_CI_NO_SCCACHE",
            "RUSTC_WRAPPER",
            "NEXTEST_EXPERIMENTAL_LIBTEST_JSON",
        )
    },
    "serialization": str(LOCK),
}
(RAW / "metadata.json").write_text(
    json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)

with LOCK.open("a+b") as lock:
    lock_wait_started_ns = time.monotonic_ns()
    fcntl.flock(lock, fcntl.LOCK_EX)
    lock_wait_ms = round((time.monotonic_ns() - lock_wait_started_ns) / 1_000_000)
    inventory_process = subprocess.run(
        INVENTORY_COMMAND,
        cwd=ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    (RAW / "inventory.stdout").write_bytes(inventory_process.stdout)
    (RAW / "inventory.stderr").write_bytes(inventory_process.stderr)
    if inventory_process.returncode != 0:
        raise SystemExit(f"inventory: exit {inventory_process.returncode}")
    if selected_inventory(inventory_process.stdout) != sorted(IDENTITIES):
        raise SystemExit("inventory: exact identity control failed")
    warmup = run("warmup", RUN_COMMAND, environment)
    samples = [run(f"sample-{index}", RUN_COMMAND, environment) for index in range(1, 4)]
    fcntl.flock(lock, fcntl.LOCK_UN)

reference_path = ROOT / "dev/benchmarks/suite-profile.json"
reference = json.loads(reference_path.read_text(encoding="utf-8"))
reference_tests = {
    test["identity"]: test["warm_duration_ms"]
    for test in reference["tests"]
    if test["identity"] in IDENTITIES
}
if set(reference_tests) != set(IDENTITIES) or sum(reference_tests.values()) != REFERENCE_MS:
    raise SystemExit("committed reference does not rederive to 64,894 ms")

implementation_parent = command_output(
    ("git", "show", "-s", "--format=%P", IMPLEMENTATION_COMMIT)
).split()[0]
implementation_patch = subprocess.check_output(
    (
        "git",
        "diff",
        "--binary",
        f"{implementation_parent}..{IMPLEMENTATION_COMMIT}",
        "--",
        *CANDIDATE_PATHS,
    ),
    cwd=ROOT,
)
accepted = all(
    sample["setup_inclusive_span_ms"] <= MAXIMUM_SPAN_MS
    and sample["reduction_from_reference_ms"] >= MINIMUM_REDUCTION_MS
    and sample["wall_ms"] <= MAXIMUM_SPAN_MS
    for sample in samples
)
summary = {
    "schema_version": 1,
    "contract": "applied-profile-library-baseline-req-04",
    "source": {
        "candidate_revision": revision,
        "candidate_tree": metadata["source_tree"],
        "implementation_commit": IMPLEMENTATION_COMMIT,
        "implementation_patch_sha256": sha256(implementation_patch),
        "candidate_paths_sha256": {
            path: sha256(subprocess.check_output(("git", "show", f"{revision}:{path}"), cwd=ROOT))
            for path in CANDIDATE_PATHS
        },
        "benchmark.py_sha256": sha256((EVIDENCE / "benchmark.py").read_bytes()),
        "validate.py_sha256": sha256((EVIDENCE / "validate.py").read_bytes()),
        "reference": {
            "artifact": "dev/benchmarks/suite-profile.json",
            "artifact_sha256": sha256(reference_path.read_bytes()),
            "artifact_commit": "58928c35d67c9f56678b577a297ad71c7584f61b",
            "profiled_revision": reference["source"]["revision"],
            "profiled_dirty": reference["source"]["dirty"],
            "method": reference["method"],
        },
    },
    "serialization": {"lock": str(LOCK), "wait_ms": lock_wait_ms},
    "environment": metadata["environment"],
    "command": list(RUN_COMMAND),
    "inventory_command": list(INVENTORY_COMMAND),
    "selected_identities": list(IDENTITIES),
    "setup_routes": {
        "default-repository-fixture": list(IDENTITIES[:2]),
        "dogfood-repository-fixture": list(IDENTITIES[2:]),
    },
    "reference_ms": REFERENCE_MS,
    "reference_tests_ms": dict(sorted(reference_tests.items())),
    "thresholds": {
        "maximum_setup_inclusive_span_ms": MAXIMUM_SPAN_MS,
        "minimum_reduction_ms": MINIMUM_REDUCTION_MS,
        "maximum_command_wall_ms": MAXIMUM_SPAN_MS,
    },
    "warmup": warmup,
    "samples": samples,
    "decision": "accepted" if accepted else "threshold_failed",
}
summary["raw_sha256"] = {
    str(path.relative_to(EVIDENCE)): sha256(path.read_bytes())
    for path in sorted(RAW.iterdir())
}
(EVIDENCE / "summary.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
print(json.dumps(summary, indent=2, sort_keys=True))
raise SystemExit(0 if accepted else 1)
