#!/usr/bin/env python3
"""Capture one fail-closed nextest-concurrency benchmark observation."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import resource
import subprocess
import sys
import threading
import time


ROOT = Path(__file__).resolve().parents[3]
SETUP_PASS = re.compile(r"SETUP PASS \[\s*([0-9.]+)s\] ([^:]+):")


def read_meminfo() -> dict[str, int]:
    values: dict[str, int] = {}
    for line in Path("/proc/meminfo").read_text(encoding="utf-8").splitlines():
        key, raw = line.split(":", 1)
        value = raw.strip().split()[0]
        if value.isdigit():
            values[key] = int(value) * 1024
    return values


def read_pressure() -> dict[str, dict[str, float | int]]:
    pressure: dict[str, dict[str, float | int]] = {}
    for resource_name in ("cpu", "memory", "io"):
        resource_pressure: dict[str, float | int] = {}
        for line in Path(f"/proc/pressure/{resource_name}").read_text(
            encoding="utf-8"
        ).splitlines():
            kind, *fields = line.split()
            for field in fields:
                key, raw = field.split("=", 1)
                resource_pressure[f"{kind}_{key}"] = (
                    int(raw) if key == "total" else float(raw)
                )
        pressure[resource_name] = resource_pressure
    return pressure


def host_snapshot() -> dict[str, object]:
    return {
        "epoch_ms": time.time_ns() // 1_000_000,
        "loadavg": list(os.getloadavg()),
        "meminfo_bytes": read_meminfo(),
        "pressure": read_pressure(),
    }


class HostMonitor:
    def __init__(self) -> None:
        initial = host_snapshot()
        meminfo = initial["meminfo_bytes"]
        loadavg = initial["loadavg"]
        assert isinstance(meminfo, dict) and isinstance(loadavg, list)
        self.started = initial
        self.minimum_mem_available_bytes = int(meminfo["MemAvailable"])
        self.minimum_swap_free_bytes = int(meminfo["SwapFree"])
        self.maximum_load_1m = float(loadavg[0])
        self.samples = 1
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._sample, daemon=True)

    def _sample(self) -> None:
        while not self.stop_event.wait(0.1):
            snapshot = host_snapshot()
            meminfo = snapshot["meminfo_bytes"]
            loadavg = snapshot["loadavg"]
            assert isinstance(meminfo, dict) and isinstance(loadavg, list)
            self.minimum_mem_available_bytes = min(
                self.minimum_mem_available_bytes, int(meminfo["MemAvailable"])
            )
            self.minimum_swap_free_bytes = min(
                self.minimum_swap_free_bytes, int(meminfo["SwapFree"])
            )
            self.maximum_load_1m = max(self.maximum_load_1m, float(loadavg[0]))
            self.samples += 1

    def start(self) -> None:
        self.thread.start()

    def finish(self) -> dict[str, object]:
        self.stop_event.set()
        self.thread.join()
        return {
            "started": self.started,
            "finished": host_snapshot(),
            "samples": self.samples,
            "minimum_mem_available_bytes": self.minimum_mem_available_bytes,
            "minimum_swap_free_bytes": self.minimum_swap_free_bytes,
            "maximum_load_1m": self.maximum_load_1m,
        }


def command_for(kind: str, workers: int | None) -> list[str]:
    if kind == "nextest":
        if workers is None:
            raise SystemExit("nextest requires --workers")
        return [
            "cargo",
            "nextest",
            "run",
            "--workspace",
            "--test-threads",
            str(workers),
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
        ]
    if kind == "inventory":
        return ["cargo", "nextest", "list", "--workspace", "--message-format", "json"]
    if kind == "doctest-list":
        return ["cargo", "test", "--doc", "--workspace", "--", "--list"]
    if kind == "doctest":
        return ["cargo", "test", "--doc", "--workspace"]
    raise AssertionError(kind)


def read_stream(stream: object, destination: Path, setup_boundary: list[int]) -> None:
    with destination.open("w", encoding="utf-8", newline="") as output:
        for line in stream:  # type: ignore[union-attr]
            output.write(line)
            output.flush()
            if SETUP_PASS.search(line):
                setup_boundary[:] = [time.monotonic_ns()]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("kind", choices=("nextest", "inventory", "doctest-list", "doctest"))
    parser.add_argument("label")
    parser.add_argument("--workers", type=int)
    args = parser.parse_args()

    output_dir = ROOT / "dev/benchmarks/nextest-concurrency-934985ca/raw" / args.label
    if output_dir.exists():
        raise SystemExit(f"refusing to overwrite observation: {output_dir}")
    output_dir.mkdir(parents=True)

    command = command_for(args.kind, args.workers)
    environment = os.environ.copy()
    environment.update(
        {"CARGO_INCREMENTAL": "0", "CARGO_CI_NO_SCCACHE": "1", "RUSTC_WRAPPER": ""}
    )
    if args.kind == "nextest":
        environment["NEXTEST_EXPERIMENTAL_LIBTEST_JSON"] = "1"

    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    started_ns = time.monotonic_ns()
    monitor = HostMonitor()
    monitor.start()
    child_usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    process = subprocess.Popen(
        command,
        cwd=ROOT,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=1,
    )
    assert process.stdout is not None and process.stderr is not None
    setup_boundary: list[int] = []
    stdout_thread = threading.Thread(
        target=read_stream, args=(process.stdout, output_dir / "stdout.log", [])
    )
    stderr_thread = threading.Thread(
        target=read_stream,
        args=(process.stderr, output_dir / "stderr.log", setup_boundary),
    )
    stdout_thread.start()
    stderr_thread.start()
    exit_code = process.wait()
    stdout_thread.join()
    stderr_thread.join()
    ended_ns = time.monotonic_ns()
    child_usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
    host = monitor.finish()

    stderr = (output_dir / "stderr.log").read_text(encoding="utf-8")
    setups = [
        {"name": match.group(2), "duration_ms": round(float(match.group(1)) * 1000)}
        for match in SETUP_PASS.finditer(stderr)
    ]
    metrics = {
        "schema_version": 1,
        "kind": args.kind,
        "label": args.label,
        "source_revision": revision,
        "workers": args.workers,
        "command": command,
        "environment": {
            key: environment.get(key)
            for key in (
                "CARGO_TARGET_DIR",
                "TMPDIR",
                "CARGO_INCREMENTAL",
                "CARGO_CI_NO_SCCACHE",
                "RUSTC_WRAPPER",
                "NEXTEST_EXPERIMENTAL_LIBTEST_JSON",
            )
        },
        "exit_code": exit_code,
        "wall_ms": round((ended_ns - started_ns) / 1_000_000),
        "setup_observations": setups,
        "setup_duration_sum_ms": sum(setup["duration_ms"] for setup in setups),
        "post_setup_region_ms": (
            round((ended_ns - setup_boundary[-1]) / 1_000_000)
            if setup_boundary
            else None
        ),
        "child_usage": {
            "maximum_resident_set_kb": child_usage_after.ru_maxrss,
            "user_cpu_seconds": child_usage_after.ru_utime - child_usage_before.ru_utime,
            "system_cpu_seconds": child_usage_after.ru_stime - child_usage_before.ru_stime,
            "major_page_faults": child_usage_after.ru_majflt - child_usage_before.ru_majflt,
            "minor_page_faults": child_usage_after.ru_minflt - child_usage_before.ru_minflt,
            "voluntary_context_switches": child_usage_after.ru_nvcsw - child_usage_before.ru_nvcsw,
            "involuntary_context_switches": child_usage_after.ru_nivcsw - child_usage_before.ru_nivcsw,
        },
        "host_pressure": host,
    }
    (output_dir / "metrics.json").write_text(
        json.dumps(metrics, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
