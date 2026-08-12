#!/usr/bin/env python3
"""Capture one warmup plus three exact four-recipe nextest samples."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = Path(__file__).resolve().parent
RAW = EVIDENCE / "raw"
REFERENCE_MS = 10_615
FILTER = " | ".join(
    (
        "test(commands::init::tests::test_single_profile_reinit_attributes_coupled_schema_repair)",
        "test(commands::init::tests::test_reinit_profiled_over_existing_root_is_idempotent_unchanged)",
        "test(failure_envelope_contract_tests::test_recorded_failure_arms_emit_the_canonical_error_envelope)",
        "test(stale_binary_child_process_tests::test_checker_child_stale_binary_fails_gate_run_visibly)",
    )
)
CONSUMERS = [
    "commands::init::tests::test_single_profile_reinit_attributes_coupled_schema_repair",
    "commands::init::tests::test_reinit_profiled_over_existing_root_is_idempotent_unchanged",
    "failure_envelope_contract_tests::test_recorded_failure_arms_emit_the_canonical_error_envelope",
    "stale_binary_child_process_tests::test_checker_child_stale_binary_fails_gate_run_visibly",
]
COMMAND = ["cargo", "nextest", "run", "--workspace", "-E", FILTER]
SETUP_PATTERN = re.compile(r"SETUP PASS \[\s*([0-9.]+)s\] ([^:]+):")


def run(label: str) -> dict[str, object]:
    process = subprocess.run(
        COMMAND,
        cwd=ROOT,
        env=os.environ,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    stdout = RAW / f"{label}.stdout"
    stderr = RAW / f"{label}.stderr"
    stdout.write_bytes(process.stdout)
    stderr.write_bytes(process.stderr)
    observations = {
        name: round(float(seconds) * 1_000)
        for seconds, name in SETUP_PATTERN.findall(process.stderr.decode("utf-8"))
    }
    if process.returncode != 0:
        raise SystemExit(f"{label}: nextest exited {process.returncode}; see {stderr}")
    expected = {
        "stale-binary-fixture",
        "default-repository-fixture",
        "dogfood-repository-fixture",
        "recorded-failure-corpus",
    }
    if set(observations) != expected:
        raise SystemExit(f"{label}: expected {expected}, observed {observations}")
    total_ms = sum(observations.values())
    return {
        "label": label,
        "stderr": str(stderr.relative_to(EVIDENCE)),
        "stdout": str(stdout.relative_to(EVIDENCE)),
        "recipes": observations,
        "total_ms": total_ms,
        "saving_ms": REFERENCE_MS - total_ms,
    }


RAW.mkdir(parents=True, exist_ok=True)
candidate_diff = subprocess.run(
    [
        "git",
        "diff",
        "--",
        "crates/jit/src/storage/mod.rs",
        "crates/jit/src/storage/test_support.rs",
        "crates/jit/src/test_utils.rs",
        "crates/jit/tests/cli_issue/failure_probe_fixture.rs",
    ],
    cwd=ROOT,
    check=True,
    stdout=subprocess.PIPE,
).stdout
warmup = run("warmup")
samples = [run(f"sample-{index}") for index in range(1, 4)]
accepted = all(
    sample["total_ms"] <= 5_500 and sample["saving_ms"] >= 5_115
    for sample in samples
)
summary = {
    "contract": "ephemeral-nextest-receipt-publication",
    "source": {
        "revision": subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=ROOT,
            check=True,
            text=True,
            stdout=subprocess.PIPE,
        ).stdout.strip(),
        "candidate_diff_sha256": hashlib.sha256(candidate_diff).hexdigest(),
        "rejected-candidate.patch.gz_sha256": hashlib.sha256(
            (EVIDENCE / "rejected-candidate.patch.gz").read_bytes()
        ).hexdigest(),
        "nextest_configuration_sha256": hashlib.sha256(
            (ROOT / ".config/nextest.toml").read_bytes()
        ).hexdigest(),
    },
    "command": COMMAND,
    "selected_consumers": CONSUMERS,
    "reference_ms": REFERENCE_MS,
    "thresholds": {"maximum_setup_ms": 5_500, "minimum_saving_ms": 5_115},
    "warmup": warmup,
    "samples": samples,
    "raw_sha256": {
        str(path.relative_to(EVIDENCE)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(RAW.iterdir())
    },
    "excluded": [
        {
            "label": "isolated-target-attempt",
            "target": "/tmp/agent-4ee95d17/target",
            "exit_code": 101,
            "reason": "disk_quota_exceeded_before_setup",
        }
    ],
    "decision": "accepted" if accepted else "no_change",
    "decision_reason": None if accepted else "quantitative_threshold_failed",
}
(EVIDENCE / "summary.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
print(json.dumps(summary, indent=2, sort_keys=True))
raise SystemExit(0 if accepted else 1)
