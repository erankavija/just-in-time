#!/usr/bin/env python3
"""Derive the read-concurrency verdict from raw/ and check summary.json.

`validate.py` rederives every metric, ratio and the verdict from `raw/`,
requires them to equal `summary.json`, and requires the verdict to pass.
`validate.py --write` writes `summary.json` from `raw/`; the harness uses it,
so the recorded summary and its check are one computation.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import statistics
import sys


EVIDENCE = Path(__file__).resolve().parent
RAW = EVIDENCE / "raw"
READER_SCALING_MIN = 4.0
WRITER_RETENTION_MIN = 0.8
MIN_LOGICAL_CPUS = 16
SCALED_READERS = 8
SINGLE_READER = 1


def load(path: Path) -> dict[str, object]:
    with path.open(encoding="utf-8") as stream:
        value = json.load(stream)
    assert isinstance(value, dict), path
    return value


def percentile(values: list[float], pct: float) -> float | None:
    """Linear interpolation between closest ranks; `None` for no values."""
    if not values:
        return None
    ordered = sorted(values)
    rank = (len(ordered) - 1) * pct
    lo = int(rank)
    hi = min(lo + 1, len(ordered) - 1)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (rank - lo)


def arm_metrics(arm: dict[str, object]) -> dict[str, object]:
    """Per-arm rates: successful completions that end inside the window."""
    window_ns = int(arm["window_ns"])
    window_s = window_ns / 1e9
    samples = arm["samples"]

    def completed(role: str) -> list[list[object]]:
        return [s for s in samples if s[0] == role and s[4] == 0 and s[3] <= window_ns]

    def failures(role: str) -> int:
        return sum(1 for s in samples if s[0] == role and s[4] != 0)

    readers = completed("reader")
    p95 = percentile([(s[3] - s[2]) / 1e6 for s in readers], 0.95)
    return {
        "round": arm["round"],
        "binary": arm["binary"],
        "readers": arm["readers"],
        "reader_per_s": round(len(readers) / window_s, 6),
        "writer_per_s": round(len(completed("writer")) / window_s, 6),
        "reader_p95_ms": None if p95 is None else round(p95, 3),
        "reader_failures": failures("reader"),
        "writer_failures": failures("writer"),
    }


def ratio(numerator: float, denominator: float) -> float | None:
    return round(numerator / denominator, 6) if denominator else None


def derive(raw: Path) -> dict[str, object]:
    host = load(raw / "host.json")
    arms = [arm_metrics(load(path)) for path in sorted((raw / "arms").glob("*.json"))]
    by_key = {(a["round"], a["binary"], a["readers"]): a for a in arms}
    assert len(by_key) == len(arms), "duplicate (round, binary, readers) arm"
    rounds = sorted({a["round"] for a in arms})
    binaries = sorted({a["binary"] for a in arms})
    expected = {
        (r, b, n) for r in rounds for b in binaries for n in host["reader_counts"]
    }
    assert set(by_key) == expected, "every round must hold every binary's arms once"
    assert len(rounds) == host["rounds"]

    per_round = [
        {
            "round": r,
            "binary": b,
            "reader_scaling": ratio(
                by_key[r, b, SCALED_READERS]["reader_per_s"],
                by_key[r, b, SINGLE_READER]["reader_per_s"],
            ),
            "writer_retention": ratio(
                by_key[r, b, SCALED_READERS]["writer_per_s"],
                by_key[r, b, 0]["writer_per_s"],
            ),
        }
        for r in rounds
        for b in binaries
    ]

    def median(binary: str, key: str) -> float | None:
        values = [p[key] for p in per_round if p["binary"] == binary]
        return None if None in values else round(statistics.median(values), 6)

    medians = {
        b: {key: median(b, key) for key in ("reader_scaling", "writer_retention")}
        for b in binaries
    }
    tip = medians.get("tip", {})
    if host["logical_cpus"] < MIN_LOGICAL_CPUS:
        verdict = "refused"
    elif (
        tip.get("reader_scaling") is not None
        and tip.get("writer_retention") is not None
        and tip["reader_scaling"] >= READER_SCALING_MIN
        and tip["writer_retention"] >= WRITER_RETENTION_MIN
    ):
        verdict = "pass"
    else:
        verdict = "fail"
    return {
        "thresholds": {
            "tip_reader_scaling_min": READER_SCALING_MIN,
            "tip_writer_retention_min": WRITER_RETENTION_MIN,
            "min_logical_cpus": MIN_LOGICAL_CPUS,
        },
        "logical_cpus": host["logical_cpus"],
        "arms": arms,
        "per_round": per_round,
        "medians": medians,
        "verdict": verdict,
    }


def recorded_summary() -> dict[str, object]:
    host = load(RAW / "host.json")
    return {
        "artifact": "dev/benchmarks/read-concurrency-bb596228/summary.json",
        "issue": "bb596228-fad0-4415-a131-54db05df0ddd",
        "binaries": host["binaries"],
        "raw_sha256": {
            str(path.relative_to(EVIDENCE)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(RAW.rglob("*"))
            if path.is_file()
        },
        **derive(RAW),
    }


if __name__ == "__main__":
    summary_path = EVIDENCE / "summary.json"
    derived = recorded_summary()
    if sys.argv[1:] == ["--write"]:
        summary_path.write_text(json.dumps(derived, indent=2) + "\n", encoding="utf-8")
        print(f"read-concurrency-bb596228: wrote summary, verdict {derived['verdict']}")
    else:
        assert sys.argv[1:] == [], "usage: validate.py [--write]"
        host = load(RAW / "host.json")
        assert (host["window_s"], host["rounds"], host["reader_counts"]) == (15, 5, [0, 1, 8])
        assert host["load_wait"]["threshold"] == 2.0
        assert float(host["load_wait"]["loadavg_at_start"][0]) < 2.0
        assert load(summary_path) == derived, "summary.json differs from raw/"
        assert derived["verdict"] == "pass", f"verdict {derived['verdict']}"
        print("read-concurrency-bb596228: evidence PASS")
