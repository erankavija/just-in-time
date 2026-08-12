#!/usr/bin/env python3
"""Recompute the dependency-profile screen decision for jit:6d10e5d4.

Reads `summary.json` beside this file, recomputes every regression against the
fixed ceilings from the arms' own recorded samples, cross-checks each recorded
sample against the retained `scripts/benchmark-rust-build.sh` output in `raw/`,
and asserts the suite bounds. Exits non-zero on the first disagreement, so the
decision this directory states is checkable without rerunning the screen.
"""

from __future__ import annotations

import json
import pathlib
import statistics
import sys

HERE = pathlib.Path(__file__).resolve().parent
RAW_ARMS = {
    "opt0_baseline": ["opt0-screen-a.json", "opt0-screen-b.json"],
    "opt1_candidate": ["opt1-screen.json"],
    "opt2_rejected_arm": ["opt2-screen-a.json", "opt2-screen-b.json"],
}
METRICS = {
    "clean_clippy_seconds": ("clean_samples", ["clippy", "wall_seconds"]),
    "clean_test_no_run_seconds": ("clean_samples", ["test_no_run", "wall_seconds"]),
    "representative_rebuild_seconds": (
        "rebuild_samples",
        ["rebuild_test_no_run", "wall_seconds"],
    ),
    "clean_target_directory_bytes": ("clean_samples", ["target_dir_bytes"]),
    "rebuild_target_directory_bytes": ("rebuild_samples", ["target_dir_bytes"]),
}
failures: list[str] = []


def check(condition: bool, message: str) -> None:
    if not condition:
        failures.append(message)


def dig(record: dict, path: list[str]):
    for key in path:
        record = record[key]
    return record


def raw_samples(files: list[str], collection: str, path: list[str]) -> list:
    return [
        dig(sample, path)
        for name in files
        for sample in json.loads((HERE / "raw" / name).read_text())[collection]
    ]


summary = json.loads((HERE / "summary.json").read_text())
screen = summary["build_screen"]
ceiling_ratio = 1 + summary["ceilings"]["maximum_regression_percent"] / 100

# Every recorded sample must be exactly what the harness wrote, in order.
for arm, files in RAW_ARMS.items():
    for metric, (collection, path) in METRICS.items():
        check(
            screen[arm][metric] == raw_samples(files, collection, path),
            f"{arm}.{metric} does not match the retained harness output",
        )

# Both ceilings are mandatory, each computed from the arms' own medians.
baseline = screen["opt0_baseline"]
selected = f"opt{summary['candidate']['selected_opt_level']}_candidate"
for metric in ("clean_clippy_seconds", "clean_test_no_run_seconds",
               "representative_rebuild_seconds"):
    base = statistics.median(baseline[metric])
    for arm in ("opt1_candidate", "opt2_rejected_arm"):
        observed = statistics.median(screen[arm][metric])
        check(
            observed <= base * ceiling_ratio,
            f"{arm}.{metric}: {observed:.3f}s exceeds the "
            f"{base * ceiling_ratio:.3f}s ceiling over {base:.3f}s",
        )

# No arm may exceed the fresh validation target-directory threshold.
for arm in RAW_ARMS:
    for metric in ("clean_target_directory_bytes", "rebuild_target_directory_bytes"):
        check(
            max(screen[arm][metric]) <= summary["ceilings"]["target_directory_bytes"],
            f"{arm}.{metric} exceeds the target-directory threshold",
        )

# The selected arm reaches every suite bound on every recorded warm run.
runtime = summary["suite_runtime"]
bounds = runtime["bounds"]
runs = runtime[selected]
check(len(runs) >= 3, "fewer than three consecutive warm runs are recorded")
for index, run in enumerate(runs, start=1):
    check(run["nextest_ms"] <= bounds["nextest_ms"], f"run {index} exceeds the nextest bound")
    check(run["doctest_ms"] <= bounds["doctest_ms"], f"run {index} exceeds the doctest bound")
    check(
        run["suite_clock_ms"] < bounds["suite_clock_ms_exclusive"],
        f"run {index} reaches or exceeds the suite-clock bound",
    )
    check(
        run["suite_clock_ms"] == run["nextest_ms"] + run["doctest_ms"],
        f"run {index}'s suite clock is not the sum of its two halves",
    )

# The recorded decision must be the one these numbers support.
check(
    summary["decision"]["value"] == "accept_opt_level_1"
    and summary["candidate"]["selected_opt_level"] == 1,
    "the recorded decision does not name the arm this evidence selects",
)
check(not summary["decision"]["thresholds_weakened"], "a threshold was weakened")

if failures:
    print("dependency-profile-6d10e5d4: FAILED", file=sys.stderr)
    for failure in failures:
        print(f"  - {failure}", file=sys.stderr)
    raise SystemExit(1)
print("dependency-profile-6d10e5d4: every recomputed assertion holds")
