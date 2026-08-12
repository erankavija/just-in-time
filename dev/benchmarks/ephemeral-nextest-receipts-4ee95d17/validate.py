#!/usr/bin/env python3
"""Recompute the ephemeral nextest receipt timing decision."""

from __future__ import annotations

import hashlib
import gzip
import json
from pathlib import Path
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = Path(__file__).resolve().parent
SETUP_PATTERN = re.compile(r"SETUP PASS \[\s*([0-9.]+)s\] ([^:]+):")


with (EVIDENCE / "summary.json").open(encoding="utf-8") as stream:
    summary = json.load(stream)

assert summary["contract"] == "ephemeral-nextest-receipt-publication"
assert summary["reference_ms"] == 10_615
assert summary["thresholds"] == {"maximum_setup_ms": 5_500, "minimum_saving_ms": 5_115}
assert summary["source"]["revision"] == "4c5aafa886c60950397fb42815dc9967d7fa98a0"
for name in ("benchmark.py", "rejected-candidate.patch.gz"):
    assert hashlib.sha256((EVIDENCE / name).read_bytes()).hexdigest() == summary["source"][f"{name}_sha256"]
assert hashlib.sha256((ROOT / ".config/nextest.toml").read_bytes()).hexdigest() == summary["source"]["nextest_configuration_sha256"]

with tempfile.TemporaryDirectory(prefix="jit-4ee95d17-candidate-") as scratch:
    clone = Path(scratch) / "repo"
    candidate_patch = Path(scratch) / "rejected-candidate.patch"
    candidate_patch.write_bytes(gzip.decompress((EVIDENCE / "rejected-candidate.patch.gz").read_bytes()))
    subprocess.run(["git", "clone", "--shared", "--quiet", str(ROOT), str(clone)], check=True)
    subprocess.run(["git", "checkout", "--quiet", summary["source"]["revision"]], cwd=clone, check=True)
    subprocess.run(["git", "apply", str(candidate_patch)], cwd=clone, check=True)
    subprocess.run(["cargo", "fmt", "--all"], cwd=clone, check=True)
    candidate_diff = subprocess.run(
        ["git", "diff", "--", *summary["candidate_paths"]],
        cwd=clone,
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
assert hashlib.sha256(candidate_diff).hexdigest() == summary["source"]["candidate_diff_sha256"]

expected_recipes = {
    "stale-binary-fixture",
    "default-repository-fixture",
    "dogfood-repository-fixture",
    "recorded-failure-corpus",
}
expected_consumers = {
    "commands::init::tests::test_single_profile_reinit_attributes_coupled_schema_repair",
    "commands::init::tests::test_reinit_profiled_over_existing_root_is_idempotent_unchanged",
    "failure_envelope_contract_tests::test_recorded_failure_arms_emit_the_canonical_error_envelope",
    "stale_binary_child_process_tests::test_checker_child_stale_binary_fails_gate_run_visibly",
}
assert set(summary["selected_consumers"]) == expected_consumers
for relative, expected_sha256 in summary["raw_sha256"].items():
    assert hashlib.sha256((EVIDENCE / relative).read_bytes()).hexdigest() == expected_sha256
binary_evidence = (EVIDENCE / "raw/candidate-binaries.txt").read_text(encoding="utf-8")
assert binary_evidence.count(f"marker={summary['candidate_binary_marker']}") == 1
assert binary_evidence.count("marker_present=true") == 2
first_sample_epoch = int(re.search(r"^first_sample_capture_epoch=(\d+)$", binary_evidence, re.MULTILINE).group(1))
artifact_mtimes = [int(value) for value in re.findall(r"^mtime_epoch=(\d+)$", binary_evidence, re.MULTILINE)]
assert len(artifact_mtimes) == 2 and all(mtime < first_sample_epoch for mtime in artifact_mtimes)
artifacts = re.findall(r"^artifact=(.+)$", binary_evidence, re.MULTILINE)
assert len(artifacts) == 2
assert artifacts[0].endswith("/target/debug/jit")
assert "/target/debug/deps/cli_issue-" in artifacts[1]
assert len(re.findall(r"^sha256=[0-9a-f]{64}$", binary_evidence, re.MULTILINE)) == 2
derived = []
for sample in summary["samples"]:
    stderr = (EVIDENCE / sample["stderr"]).read_text(encoding="utf-8")
    assert all(re.search(rf"PASS .*{re.escape(identity)}$", stderr, re.MULTILINE) for identity in expected_consumers)
    observations = {
        name: round(float(seconds) * 1_000)
        for seconds, name in SETUP_PATTERN.findall(stderr)
    }
    assert set(observations) == expected_recipes
    total_ms = sum(observations.values())
    saving_ms = summary["reference_ms"] - total_ms
    assert observations == sample["recipes"]
    assert total_ms == sample["total_ms"]
    assert saving_ms == sample["saving_ms"]
    assert total_ms > summary["thresholds"]["maximum_setup_ms"]
    assert saving_ms < summary["thresholds"]["minimum_saving_ms"]
    derived.append(total_ms)

assert len(derived) == 3
assert summary["decision"] == "no_change"
assert summary["decision_reason"] == "quantitative_threshold_failed"
assert summary["candidate_binary_marker"] == "nextest receipt environment name is invalid"
assert summary["excluded"][0]["reason"] == "disk_quota_exceeded_before_setup"
print("ephemeral-nextest-receipts-4ee95d17: evidence PASS")
