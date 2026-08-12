#!/usr/bin/env python3
"""Rederive ae17798d's identity, provenance, and REQ-04 arithmetic."""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = Path(__file__).resolve().parent
SETUP_PATTERN = re.compile(r"SETUP PASS \[\s*([0-9.]+)s\] ([^:]+):")
EXPECTED_IDENTITIES = {
    "jit::jit$commands::init::tests::test_single_profile_reinit_attributes_coupled_schema_repair",
    "jit::jit$commands::profile::tests::test_profile_preparation_retries_when_final_proposed_closure_expands",
    "jit::jit$commands::init::tests::test_reinit_profiled_over_existing_root_is_idempotent_unchanged",
    "jit::jit$commands::profile::tests::test_apply_profile_selection_recovers_an_obsolete_default_schema_atomically",
    "jit::jit$commands::validate::tests::test_validate_fix_repairs_every_owned_materialization_and_preserves_unowned_files",
    "jit::jit$commands::validate::tests::test_validate_fix_rejects_ambiguous_region_without_writing_other_repairs",
    "jit::jit$commands::validate::tests::test_validate_fix_repairs_owned_materializations_in_memory",
    "jit::jit$commands::validate::tests::test_validate_fix_rejects_ambiguous_region_without_memory_writes",
    "jit::jit$commands::validate::tests::test_capture_repair_plan_with_recorded_profile_captures_and_repairs_its_targets",
    "jit::jit$commands::validate::tests::test_validate_diagnoses_non_profile_drift_identically_across_record_states",
}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load(path: Path) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    assert isinstance(value, dict)
    return value


def events(path: Path) -> list[dict[str, object]]:
    observed = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "test":
            observed.append(event)
    return observed


summary = load(EVIDENCE / "summary.json")
assert summary["schema_version"] == 1
assert summary["contract"] == "applied-profile-library-baseline-req-04"
assert summary["source"]["candidate_revision"] == "db82ac6cc5344d8d8343e76f058e71f8c55812c4"
assert summary["source"]["implementation_commit"] == "61234ddf0962536303e0447e0a8e530d2473a9c6"
assert summary["source"]["reference"]["artifact_commit"] == "58928c35d67c9f56678b577a297ad71c7584f61b"
assert summary["source"]["reference"]["profiled_revision"] == "6c536622212d2790ad624f3a154e80b82f7fbbe8"
assert summary["source"]["reference"]["profiled_dirty"] is False
assert summary["reference_ms"] == 64_894
assert summary["thresholds"] == {
    "maximum_command_wall_ms": 25_000,
    "maximum_setup_inclusive_span_ms": 25_000,
    "minimum_reduction_ms": 25_000,
}
assert set(summary["selected_identities"]) == EXPECTED_IDENTITIES
assert len(summary["selected_identities"]) == len(EXPECTED_IDENTITIES) == 10
assert set(summary["setup_routes"]["default-repository-fixture"]) == set(
    summary["selected_identities"][:2]
)
assert set(summary["setup_routes"]["dogfood-repository-fixture"]) == set(
    summary["selected_identities"][2:]
)

reference_path = ROOT / summary["source"]["reference"]["artifact"]
assert sha256(reference_path.read_bytes()) == summary["source"]["reference"]["artifact_sha256"]
reference = load(reference_path)
assert reference["source"] == {
    "dirty": False,
    "revision": summary["source"]["reference"]["profiled_revision"],
}
reference_tests = {
    test["identity"]: test["warm_duration_ms"]
    for test in reference["tests"]
    if test["identity"] in EXPECTED_IDENTITIES
}
assert set(reference_tests) == EXPECTED_IDENTITIES
assert reference_tests == summary["reference_tests_ms"]
assert sum(reference_tests.values()) == 64_894

for script in ("benchmark.py", "validate.py"):
    assert sha256((EVIDENCE / script).read_bytes()) == summary["source"][f"{script}_sha256"]
for path, expected_sha256 in summary["source"]["candidate_paths_sha256"].items():
    content = subprocess.check_output(
        ("git", "show", f"{summary['source']['candidate_revision']}:{path}"), cwd=ROOT
    )
    assert sha256(content) == expected_sha256
parent = subprocess.check_output(
    ("git", "show", "-s", "--format=%P", summary["source"]["implementation_commit"]),
    cwd=ROOT,
    text=True,
).split()[0]
patch = subprocess.check_output(
    (
        "git",
        "diff",
        "--binary",
        f"{parent}..{summary['source']['implementation_commit']}",
        "--",
        *summary["source"]["candidate_paths_sha256"].keys(),
    ),
    cwd=ROOT,
)
assert sha256(patch) == summary["source"]["implementation_patch_sha256"]

for relative, expected_sha256 in summary["raw_sha256"].items():
    assert sha256((EVIDENCE / relative).read_bytes()) == expected_sha256

inventory = load(EVIDENCE / "raw/inventory.stdout")
inventory_identities = []
for binary, suite in inventory["rust-suites"].items():
    for test_name, test in suite["testcases"].items():
        if test["filter-match"]["status"] == "matches":
            inventory_identities.append(f"{suite['package-name']}::{binary}${test_name}")
assert set(inventory_identities) == EXPECTED_IDENTITIES
assert len(inventory_identities) == len(set(inventory_identities)) == 10

metadata = load(EVIDENCE / "raw/metadata.json")
assert metadata["source_revision"] == summary["source"]["candidate_revision"]
assert metadata["source_tree"] == summary["source"]["candidate_tree"]
assert metadata["serialization"] == summary["serialization"]["lock"] == "/tmp/cargo-ci.lock"
assert metadata["environment"] == summary["environment"]
assert summary["environment"] == {
    "CARGO_CI_NO_SCCACHE": "1",
    "CARGO_INCREMENTAL": "0",
    "CARGO_TARGET_DIR": "/home/vkaskivuo/Projects/just-in-time/target",
    "NEXTEST_EXPERIMENTAL_LIBTEST_JSON": "1",
    "RUSTC_WRAPPER": "",
}

for run in (summary["warmup"], *summary["samples"]):
    observed_events = events(EVIDENCE / run["stdout"])
    started = [event["name"] for event in observed_events if event.get("event") == "started"]
    results = [
        event
        for event in observed_events
        if event.get("event") in ("ok", "failed", "ignored")
    ]
    assert set(started) == EXPECTED_IDENTITIES
    assert len(started) == len(set(started)) == 10
    assert {event["name"] for event in results} == EXPECTED_IDENTITIES
    assert len(results) == len({event["name"] for event in results}) == 10
    assert all(event["event"] == "ok" for event in results)
    tests_ms = {
        event["name"]: math.ceil(float(event["exec_time"]) * 1_000)
        for event in results
    }
    setup_matches = SETUP_PATTERN.findall(
        (EVIDENCE / run["stderr"]).read_text(encoding="utf-8")
    )
    setups_ms = {
        name: round(float(seconds) * 1_000) for seconds, name in setup_matches
    }
    assert len(setup_matches) == 2
    assert set(setups_ms) == {
        "default-repository-fixture",
        "dogfood-repository-fixture",
    }
    assert tests_ms == run["tests_ms"]
    assert setups_ms == run["setups_ms"]
    assert sum(tests_ms.values()) == run["test_sum_ms"]
    assert sum(setups_ms.values()) == run["setup_sum_ms"]
    span_ms = sum(tests_ms.values()) + sum(setups_ms.values())
    assert span_ms == run["setup_inclusive_span_ms"]
    assert 64_894 - span_ms == run["reduction_from_reference_ms"]
    assert run["exit_code"] == 0

for sample in summary["samples"]:
    assert sample["setup_inclusive_span_ms"] <= 25_000
    assert sample["reduction_from_reference_ms"] >= 25_000
    assert sample["wall_ms"] <= 25_000
assert len(summary["samples"]) == 3
assert summary["decision"] == "accepted"
print("applied-profile-baselines-ae17798d: evidence PASS")
