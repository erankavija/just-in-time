#!/usr/bin/env python3
"""Recompute the nextest concurrency evidence's semantic invariants."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = Path(__file__).resolve().parent
RAW = EVIDENCE / "raw"


def load(path: Path) -> dict[str, object]:
    with path.open(encoding="utf-8") as stream:
        value = json.load(stream)
    assert isinstance(value, dict)
    return value


def test_events(label: str) -> list[dict[str, object]]:
    events = []
    for line in (RAW / label / "stdout.log").read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "test":
            events.append(event)
    return events


summary = load(EVIDENCE / "summary.json")
assert summary["contract"] == "nextest-worker-concurrency-benchmark"
assert summary["source"]["revision"] == "c1d072df3ad3aafd60055bb25b4c1f1b70b49197"
assert summary["decision"]["value"] == "no_change"
assert summary["decision"]["accepted_workers"] is None
assert summary["thresholds"]["weakened"] is False

inventory = load(RAW / "inventory" / "stdout.log")
identities = []
for binary, suite in inventory["rust-suites"].items():
    for test_name, test in suite["testcases"].items():
        identities.append(
            f"{suite['package-name']}::{binary}${test_name}\0{str(test['ignored']).lower()}"
        )
assert len(identities) == 4521
assert len(set(identities)) == 4521
assert sum(identity.endswith("\0false") for identity in identities) == 4511
assert sum(identity.endswith("\0true") for identity in identities) == 10
identity_digest = hashlib.sha256(
    ("\n".join(sorted(identities)) + "\n").encode()
).hexdigest()
assert identity_digest == summary["identity_control"]["normalized_sha256"]

doctest_lines = (RAW / "doctest-inventory" / "stdout.log").read_text(
    encoding="utf-8"
).splitlines()
doctests = [line.removesuffix(": test") for line in doctest_lines if line.endswith(": test")]
assert len(doctests) == len(set(doctests)) == 63
doctest_digest = hashlib.sha256(
    ("\n".join(sorted(doctests)) + "\n").encode()
).hexdigest()
assert doctest_digest == summary["identity_control"]["doctest_normalized_sha256"]

for expected in summary["runs"]:
    label = expected["label"]
    metrics = load(RAW / label / "metrics.json")
    assert metrics["source_revision"] == summary["source"]["revision"]
    for key, metric_key in (
        ("workers", "workers"),
        ("exit_code", "exit_code"),
        ("wall_ms", "wall_ms"),
        ("setup_ms", "setup_duration_sum_ms"),
        ("post_setup_ms", "post_setup_region_ms"),
    ):
        assert expected[key] == metrics[metric_key]
    setup = {item["name"]: item["duration_ms"] for item in metrics["setup_observations"]}
    assert setup == expected["setup"]

    events = test_events(label)
    started = [event["name"] for event in events if event.get("event") == "started"]
    results = [event for event in events if event.get("event") in ("ok", "failed", "ignored")]
    passed = [event for event in results if event["event"] == "ok"]
    failed = [event for event in results if event["event"] == "failed"]
    ignored = [event for event in results if event["event"] == "ignored"]
    assert len(started) - len(set(started)) == expected["duplicate_started"]
    assert len(results) - len({event["name"] for event in results}) == expected["duplicate_results"]
    assert len(passed) == expected["passed"]
    assert len(failed) == expected["failed"]
    assert len(ignored) == expected["ignored"]
    assert 4511 - len(passed) - len(failed) == expected["not_run"]
    aggregate = sum(float(event["exec_time"]) for event in passed)
    aggregate_key = (
        "aggregate_test_work_seconds"
        if expected["failed"] == 0
        else "aggregate_successful_test_work_seconds"
    )
    assert abs(aggregate - expected[aggregate_key]) < 1e-9
    maximum = max(float(event["exec_time"]) for event in passed)
    maximum_key = "maximum_test_seconds" if expected["failed"] == 0 else "maximum_successful_test_seconds"
    assert abs(maximum - expected[maximum_key]) < 1e-9
    if failed:
        assert "jit-bootstrap.lock" in str(failed[0]["stdout"])

doctest_metrics = load(RAW / "doctest-1" / "metrics.json")
assert doctest_metrics["exit_code"] == 0
assert doctest_metrics["wall_ms"] == summary["doctest_control"]["wall_ms"]
doctest_output = (RAW / "doctest-1" / "stdout.log").read_text(encoding="utf-8")
assert re.search(r"test result: ok\. 63 passed; 0 failed; 0 ignored", doctest_output)

config_bytes = (ROOT / ".config/nextest.toml").read_bytes()
assert hashlib.sha256(config_bytes).hexdigest() == summary["source"]["nextest_configuration_sha256"]
assert summary["runs"][2]["wall_ms"] > summary["thresholds"]["maximum_each_full_run_ms"]
assert summary["runs"][2]["maximum_test_seconds"] > summary["thresholds"]["maximum_test_seconds"]
assert summary["runs"][3]["failed"] == 1
assert summary["decision"]["nextest_configuration_changed"] is False
print("nextest-concurrency-934985ca: evidence PASS")
