#!/usr/bin/env python3
"""Prove the validator's derivation on synthetic raw arms."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sys
import tempfile


EVIDENCE = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "read_concurrency_validator", EVIDENCE / "validate.py"
)
assert SPEC is not None and SPEC.loader is not None
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

WINDOW_NS = 1_000_000_000


def loop(role: str, worker: int, count: int, rc: int = 0) -> list[list[object]]:
    """`count` back-to-back completions spread evenly over one window."""
    step = WINDOW_NS // count
    return [[role, worker, i * step, (i + 1) * step, rc] for i in range(count)]


def arm(binary: str, readers: int, reader_total: int, writer: int) -> dict[str, object]:
    samples = loop("writer", 0, writer)
    for worker in range(readers):
        samples += loop("reader", worker, reader_total // readers)
    return {"binary": binary, "readers": readers, "window_ns": WINDOW_NS, "samples": samples}


def write_raw(root: Path, cpus: int, tip_scaled: bool) -> Path:
    raw = root / "raw"
    (raw / "arms").mkdir(parents=True)
    (raw / "host.json").write_text(
        json.dumps({"logical_cpus": cpus, "rounds": 1, "reader_counts": [0, 1, 8]})
    )
    arms = [
        arm("base", 0, 0, 40),
        arm("base", 1, 20, 20),
        arm("base", 8, 24, 10),
        arm("tip", 0, 0, 40),
        arm("tip", 1, 20, 40),
        arm("tip", 8, 160 if tip_scaled else 24, 36),
    ]
    for seq, value in enumerate(arms):
        value["round"] = 0
        (raw / "arms" / f"{seq:03d}.json").write_text(json.dumps(value))
    return raw


def test_derive_passes_scaled_tip_and_records_serialized_base() -> None:
    with tempfile.TemporaryDirectory() as scratch:
        summary = validator.derive(write_raw(Path(scratch), 24, tip_scaled=True))
    assert summary["verdict"] == "pass"
    assert summary["medians"]["tip"]["reader_scaling"] == 8.0
    assert summary["medians"]["tip"]["writer_retention"] == 0.9
    assert summary["medians"]["base"]["reader_scaling"] < 4


def test_derive_fails_serialized_tip() -> None:
    with tempfile.TemporaryDirectory() as scratch:
        summary = validator.derive(write_raw(Path(scratch), 24, tip_scaled=False))
    assert summary["verdict"] == "fail"


def test_derive_refuses_verdict_below_sixteen_cpus() -> None:
    with tempfile.TemporaryDirectory() as scratch:
        summary = validator.derive(write_raw(Path(scratch), 15, tip_scaled=True))
    assert summary["verdict"] == "refused"


def test_arm_metrics_count_only_successes_inside_the_window() -> None:
    samples = loop("reader", 0, 4) + [
        ["reader", 0, 0, WINDOW_NS // 2, 75],
        ["reader", 0, WINDOW_NS - 1, WINDOW_NS + 1, 0],
    ]
    metrics = validator.arm_metrics(
        {"binary": "tip", "readers": 1, "round": 0, "window_ns": WINDOW_NS, "samples": samples}
    )
    assert metrics["reader_per_s"] == 4.0
    assert metrics["reader_failures"] == 1


for name, test in sorted(globals().items()):
    if name.startswith("test_") and callable(test):
        test()
print("read-concurrency-bb596228: validator selftest PASS")
