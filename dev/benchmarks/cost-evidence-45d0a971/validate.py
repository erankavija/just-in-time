#!/usr/bin/env python3
"""Derive the REQ-22 cost verdicts from raw/ and check summary.json.

`validate.py` rederives every median, slope, ratio and verdict from `raw/base/`
and `raw/tip/`, requires them to equal `summary.json`, requires the base to
reproduce each cost in both runs and every tip verdict to pass.
`validate.py --write` writes `summary.json` from `raw/`; the harness
(`scripts/benchmark-cost-evidence.sh`) uses it, so the recorded summary and its
check are one computation. `validate.py --raw <dir>` prints one run's derivation.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import statistics
import sys


EVIDENCE = Path(__file__).resolve().parent
RAW = EVIDENCE / "raw"
BASE_COMMIT = "369d35a42f0ecc5067d930415c44908defdc0e36"
SAMPLES = 5
LOAD_MAX = 2.0
THRESHOLDS = {
    "bulk_tip_slope_ratio_max": 1.5,
    "archive_tip_slope_over_base_max": 0.5,
    "dep_tip_d_over_base_max": 0.7,
    "profile_tip_dir_opens_over_base_max": 0.6,
    "bulk_base_slope_ratio_min": 2.5,
}
BULK_MATCHES = (10, 160)
ARCHIVE_CANDIDATES = (10, 40)


def load(path: Path) -> object:
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def ratio(numerator: float, denominator: float) -> float | None:
    return round(numerator / denominator, 6) if denominator else None


def trace_counts(rows: list[list[str]]) -> dict[str, int]:
    """Counts over one trace's classified rows `<call> <kind> <scope> <path>`."""
    opens = [r for r in rows if r[0] == "open"]
    syncs = [r for r in rows if r[0] in ("fsync", "fdatasync")]
    return {
        "config_opens": sum(1 for r in opens if r[3] == ".jit/config.toml"),
        "dir_opens": sum(1 for r in opens if r[1] == "dir"),
        "dir_opens_inside": sum(1 for r in opens if r[1] == "dir" and r[2] == "inside"),
        "dir_opens_outside": sum(1 for r in opens if r[1] == "dir" and r[2] == "outside"),
        "fsyncs": len(syncs),
        "fsyncs_dir": sum(1 for r in syncs if r[1] == "dir"),
        "fsyncs_file": sum(1 for r in syncs if r[1] == "file"),
    }


def derive(raw: Path) -> dict[str, object]:
    host = load(raw / "host.json")
    timings = load(raw / "timings.json")
    traces = load(raw / "traces.json")
    binaries = list(host["binaries"])
    rounds = range(host["samples"])

    wall: dict[tuple[str, str], list[float]] = {}
    for t in timings:
        wall.setdefault((t["binary"], t["point"]), []).append(t["wall_ns"] / 1e6)
    counted: dict[tuple[str, str], list[dict[str, int]]] = {}
    for t in traces:
        text = (raw / "traces" / t["trace"]).read_text(encoding="utf-8")
        rows = [line.split("\t", 3) for line in text.splitlines()]
        counted.setdefault((t["binary"], t["point"]), []).append(trace_counts(rows))
    for key, values in [*wall.items(), *counted.items()]:
        assert len(values) == len(rounds), f"{key}: {len(values)} samples, want {len(rounds)}"

    def ms(binary: str, point: str) -> float:
        return round(statistics.median(wall[binary, point]), 3)

    def count(binary: str, point: str, key: str) -> float:
        return statistics.median(c[key] for c in counted[binary, point])

    def slope(binary: str, low: str, high: str, span: int) -> float:
        return round((ms(binary, high) - ms(binary, low)) / span, 4)

    bulk_low, bulk_high = BULK_MATCHES
    bulk_span = bulk_high - bulk_low
    archive_low, archive_high = ARCHIVE_CANDIDATES

    metrics = {}
    for b in binaries:
        s500 = slope(b, f"bulk/N500-M{bulk_low}", f"bulk/N500-M{bulk_high}", bulk_span)
        s2000 = slope(b, f"bulk/N2000-M{bulk_low}", f"bulk/N2000-M{bulk_high}", bulk_span)
        profile = {key: count(b, "profile/package48", key) for key in (
            "dir_opens", "dir_opens_inside", "dir_opens_outside",
            "fsyncs", "fsyncs_dir", "fsyncs_file")}
        metrics[b] = {
            "median_ms": {point: ms(b, point) for (binary, point) in sorted(wall)
                          if binary == b},
            "bulk": {"s500_ms": s500, "s2000_ms": s2000, "slope_ratio": ratio(s2000, s500)},
            "archive": {
                "per_candidate_ms": slope(b, f"archive/C{archive_low}", f"archive/C{archive_high}",
                                      archive_high - archive_low),
                "config_opens_c10": count(b, "archive/C10", "config_opens"),
                "config_opens_c40": count(b, "archive/C40", "config_opens"),
            },
            "dep": {"declaration_ms": round(ms(b, "dep/inflated") - ms(b, "dep/minimal"), 3)},
            "profile": profile,
        }

    base = metrics["base"]
    reproduction = {
        "bulk_slope_ratio_at_least_min": base["bulk"]["slope_ratio"] is not None
        and base["bulk"]["slope_ratio"] >= THRESHOLDS["bulk_base_slope_ratio_min"],
        "config_opens_grow_with_candidates":
            base["archive"]["config_opens_c40"] > base["archive"]["config_opens_c10"],
        "declaration_time_positive": base["dep"]["declaration_ms"] > 0,
    }
    result = {"metrics": metrics, "base_reproduces": reproduction}
    if "tip" in metrics:
        tip = metrics["tip"]
        archive_ratio = ratio(tip["archive"]["per_candidate_ms"],
                              base["archive"]["per_candidate_ms"])
        dep_ratio = ratio(tip["dep"]["declaration_ms"], base["dep"]["declaration_ms"])
        profile_ratio = ratio(tip["profile"]["dir_opens"], base["profile"]["dir_opens"])
        result["ratios"] = {
            "bulk_tip_slope_ratio": tip["bulk"]["slope_ratio"],
            "archive_tip_slope_over_base": archive_ratio,
            "dep_tip_d_over_base": dep_ratio,
            "profile_tip_dir_opens_over_base": profile_ratio,
        }
        result["verdicts"] = {
            "bulk": tip["bulk"]["slope_ratio"] is not None
            and tip["bulk"]["slope_ratio"] <= THRESHOLDS["bulk_tip_slope_ratio_max"],
            "archive": archive_ratio is not None
            and archive_ratio <= THRESHOLDS["archive_tip_slope_over_base_max"]
            and tip["archive"]["config_opens_c10"] == tip["archive"]["config_opens_c40"],
            "dep": dep_ratio is not None
            and base["dep"]["declaration_ms"] > 0
            and dep_ratio <= THRESHOLDS["dep_tip_d_over_base_max"],
            "profile": profile_ratio is not None
            and profile_ratio <= THRESHOLDS["profile_tip_dir_opens_over_base_max"],
        }
        result["verdicts"] = {k: "pass" if v else "fail" for k, v in result["verdicts"].items()}
    return result


def recorded_summary() -> dict[str, object]:
    runs = {label: RAW / label for label in ("base", "tip") if (RAW / label).is_dir()}
    return {
        "artifact": "dev/benchmarks/cost-evidence-45d0a971/summary.json",
        "issue": "45d0a971-b44a-4a32-8f09-2553978cf0bf",
        "thresholds": THRESHOLDS,
        "runs": {
            label: {"binaries": load(path / "host.json")["binaries"], **derive(path)}
            for label, path in runs.items()
        },
        "raw_sha256": {
            str(path.relative_to(EVIDENCE)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in sorted(RAW.rglob("*"))
            if path.is_file()
        },
    }


def check() -> None:
    summary = load(EVIDENCE / "summary.json")
    derived = recorded_summary()
    assert set(derived["runs"]) == {"base", "tip"}, "raw/base/ and raw/tip/ are both required"
    for label in ("base", "tip"):
        host = load(RAW / label / "host.json")
        assert host["label"] == label
        assert host["samples"] == SAMPLES, f"{label}: recorded with {host['samples']} rounds"
        assert host["load_wait"]["threshold"] == LOAD_MAX
        assert float(host["loadavg_at_start"][0]) < LOAD_MAX
        assert host["binaries"]["base"]["commit"] == BASE_COMMIT
        assert all(derived["runs"][label]["base_reproduces"].values()), \
            f"{label}: base does not reproduce {derived['runs'][label]['base_reproduces']}"
    tip_host = load(RAW / "tip" / "host.json")
    assert set(tip_host["binaries"]) == {"base", "tip"}
    assert summary == derived, "summary.json differs from raw/"
    verdicts = derived["runs"]["tip"]["verdicts"]
    failed = {k: v for k, v in verdicts.items() if v != "pass"}
    assert not failed, f"failing verdicts {failed}, ratios {derived['runs']['tip']['ratios']}"
    print("cost-evidence-45d0a971: evidence PASS")


if __name__ == "__main__":
    if sys.argv[1:] == ["--write"]:
        derived = recorded_summary()
        (EVIDENCE / "summary.json").write_text(json.dumps(derived, indent=2) + "\n",
                                               encoding="utf-8")
        verdicts = derived["runs"].get("tip", {}).get("verdicts", {})
        print(f"cost-evidence-45d0a971: wrote summary, verdicts {verdicts or 'base-only'}")
    elif len(sys.argv) == 3 and sys.argv[1] == "--raw":
        print(json.dumps(derive(Path(sys.argv[2])), indent=2))
    else:
        assert sys.argv[1:] == [], "usage: validate.py [--write | --raw <dir>]"
        check()
