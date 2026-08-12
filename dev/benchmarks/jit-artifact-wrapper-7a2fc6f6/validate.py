#!/usr/bin/env python3
"""Recompute the durable no-change evidence for jit:7a2fc6f6."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import subprocess


HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def load_jsonl_one(path: pathlib.Path):
    records = [json.loads(line) for line in path.read_text().splitlines() if line]
    assert len(records) == 1
    return records[0]


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory_identities(path: pathlib.Path):
    inventory = load(path)
    identities = sorted(
        (
            target["name"],
            ",".join(target["kind"]),
            test["name"],
            bool(test["ignored"]),
        )
        for target in inventory["targets"]
        for test in target["tests"]
    )
    encoded = b"".join(
        ("\0".join((*identity[:3], str(identity[3]).lower())) + "\n").encode()
        for identity in identities
    )
    return identities, hashlib.sha256(encoded).hexdigest()


def validate() -> None:
    summary = load(HERE / "summary.json")
    audit = load(HERE / "compiler-audit.json")
    baseline = load(HERE / "screen/opt0-build/baseline.json")
    candidate = load_jsonl_one(
        HERE / "screen/candidate-build/raw/clean-samples.jsonl"
    )

    assert summary["source"]["revision"] == summary["source"]["required_revision"]
    assert summary["source"]["required_revision"] == (
        "31e7d502f92a12b0565a41a4ed37c90860097455"
    )
    assert sha256(HERE / "rejected-candidate.patch") == summary["source"][
        "candidate_patch_sha256"
    ]
    assert summary["decision"]["value"] == "no_change"
    assert summary["decision"]["candidate_accepted"] is False
    assert summary["decision"]["integration_retained"] is False
    assert summary["decision"]["thresholds_weakened"] is False

    baseline_clean = baseline["medians"]["clean_test_no_run_wall_seconds"]
    baseline_rebuild = baseline["medians"]["rebuild_wall_seconds"]
    candidate_clean = candidate["test_no_run"]["wall_seconds"]
    assert baseline_clean == summary["build_screen"][
        "baseline_clean_test_build_seconds"
    ]
    assert baseline_rebuild == summary["build_screen"][
        "baseline_representative_rebuild_seconds"
    ]
    assert candidate_clean == summary["build_screen"][
        "candidate_clean_test_build_seconds"
    ]
    assert round(baseline_clean * 1.25, 6) == summary["build_screen"][
        "clean_build_ceiling_seconds"
    ]
    assert candidate_clean > baseline_clean * 1.25
    assert summary["build_screen"]["candidate_representative_rebuild_seconds"] is None
    assert summary["acceptance_only_runtime"] == {
        "cargo_ci_runs": 0,
        "doctest_samples": 0,
        "nextest_samples": 0,
        "status": "not_run_after_hard_clean_build_rejection",
    }

    baseline_ids, baseline_digest = inventory_identities(
        HERE / "screen/opt0-build/pre-change-test-inventory.json"
    )
    candidate_ids, candidate_digest = inventory_identities(
        HERE / "screen/candidate-build/pre-change-test-inventory.json"
    )
    assert baseline_ids == candidate_ids
    assert baseline_digest == candidate_digest == summary["identity"][
        "normalized_sha256"
    ]
    regular = [identity for identity in baseline_ids if identity[1] != "doctest"]
    doctests = [identity for identity in baseline_ids if identity[1] == "doctest"]
    assert sum(not identity[3] for identity in regular) == summary["identity"][
        "regular_runnable"
    ]
    assert sum(identity[3] for identity in regular) == summary["identity"][
        "regular_ignored"
    ]
    assert len(doctests) == summary["identity"]["doctest_count"]

    assert audit["record_count"] == len(audit["records"]) == 25
    optimized = [r for r in audit["records"] if r["decision"] == "optimize"]
    passthrough = [r for r in audit["records"] if r["decision"] == "passthrough"]
    assert len(optimized) == audit["optimized_count"] == 2
    assert len(passthrough) == audit["passthrough_count"] == 23
    assert audit["conflicting_opt_level_count"] == 0
    assert {
        pathlib.PurePath(r["root"]).as_posix().split("/crates/", 1)[1]
        for r in optimized
    } == {"jit/src/lib.rs", "jit/src/main.rs"}
    assert all(
        r["package"] == "jit"
        and r["crate"] == "jit"
        and r["has_test"] == "0"
        and r["opt_level_count"] == "0"
        for r in optimized
    )
    assert all(r["decision"] == "passthrough" for r in passthrough)

    assert candidate["inventory"]["unique_active_test_executable_bytes"] == summary[
        "size_screen"
    ]["candidate_active_test_executable_bytes"]
    assert candidate["target_dir_bytes"] == summary["size_screen"][
        "candidate_target_dir_bytes"
    ]
    assert summary["size_screen"]["passed"] is True

    line_table = (HERE / "raw/line-table.txt").read_text(encoding="utf-8")
    backtrace = (HERE / "raw/backtrace.stderr").read_text(encoding="utf-8")
    selftest = (HERE / "raw/wrapper-selftest.stdout").read_text(encoding="utf-8")
    interruption = (HERE / "raw/candidate-session-interruption.txt").read_text(
        encoding="utf-8"
    )
    assert ".debug_line" in line_table and "main.rs" in line_table and "2214" in line_table
    assert "at ./crates/jit/src/main.rs:2214:" in backtrace
    assert "SELFTEST: all assertions passed" in selftest
    assert "harness exit code: 130" in interruption
    assert "before the representative source probe was applied" in interruption

    # The measured integration is intentionally absent after the hard reject.
    for rejected_path in [
        ROOT / "scripts/jit-rustc-workspace-wrapper.sh",
        ROOT / "scripts/stage-jit-rustc-workspace-wrapper.sh",
        ROOT / "scripts/jit-rustc-workspace-wrapper-selftest.sh",
        ROOT / "crates/jit/tests/scratch_build/jit_rustc_workspace_wrapper_tests.rs",
    ]:
        assert not rejected_path.exists(), rejected_path
    assert "RUSTC_WORKSPACE_WRAPPER" not in (
        ROOT / "scripts/cargo-ci.sh"
    ).read_text(encoding="utf-8")
    for restored_path in [
        "crates/jit/tests/scratch_build/build_profile_policy_tests.rs",
        "crates/jit/tests/scratch_build/main.rs",
        "scripts/benchmark-rust-build.sh",
        "scripts/cargo-ci.sh",
    ]:
        base_bytes = subprocess.run(
            [
                "git",
                "-C",
                str(ROOT),
                "show",
                f"{summary['source']['required_revision']}:{restored_path}",
            ],
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
        assert (ROOT / restored_path).read_bytes() == base_bytes, restored_path


def write_manifest() -> None:
    files = sorted(
        path
        for path in HERE.rglob("*")
        if path.is_file() and path.name != "SHA256SUMS"
    )
    lines = [f"{sha256(path)}  {path.relative_to(HERE).as_posix()}" for path in files]
    destination = HERE / "SHA256SUMS"
    temporary = destination.with_name(f".SHA256SUMS.tmp.{os.getpid()}")
    temporary.write_text("\n".join(lines) + "\n", encoding="utf-8")
    temporary.replace(destination)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write-manifest", action="store_true")
    args = parser.parse_args()
    validate()
    if args.write_manifest:
        write_manifest()
    print("jit-artifact-wrapper-7a2fc6f6: evidence PASS")


if __name__ == "__main__":
    main()
