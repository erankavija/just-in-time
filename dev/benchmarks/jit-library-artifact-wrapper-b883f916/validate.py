#!/usr/bin/env python3
"""Recompute durable no-change evidence for jit:b883f916."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import tempfile


HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
EXPECTED_PATCH_PATHS = {
    "crates/jit/tests/scratch_build/build_profile_policy_tests.rs",
    "crates/jit/tests/scratch_build/jit_rustc_workspace_wrapper_tests.rs",
    "crates/jit/tests/scratch_build/main.rs",
    "scripts/benchmark-rust-build.sh",
    "scripts/cargo-ci.sh",
    "scripts/jit-rustc-workspace-wrapper-selftest.sh",
    "scripts/jit-rustc-workspace-wrapper.sh",
    "scripts/stage-jit-rustc-workspace-wrapper.sh",
}


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


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


def validate_rejected_patch(required_revision: str) -> None:
    patch = HERE / "rejected-candidate.patch"
    patch_text = patch.read_text(encoding="utf-8")
    patch_paths = {
        line.split(" b/", 1)[1]
        for line in patch_text.splitlines()
        if line.startswith("diff --git a/")
    }
    assert patch_paths == EXPECTED_PATCH_PATHS
    for executable in [
        "scripts/jit-rustc-workspace-wrapper-selftest.sh",
        "scripts/jit-rustc-workspace-wrapper.sh",
        "scripts/stage-jit-rustc-workspace-wrapper.sh",
    ]:
        marker = f"diff --git a/{executable} b/{executable}"
        section = patch_text.split(marker, 1)[1].split("diff --git ", 1)[0]
        assert "new file mode 100755" in section

    with tempfile.TemporaryDirectory(prefix="jit-b883f916-patch-") as temporary:
        reconstructed = pathlib.Path(temporary) / "source"
        reconstructed.mkdir()
        # Export exactly the measured revision without depending on mutable
        # working-tree bytes, then apply the plain durable patch.
        archive = subprocess.Popen(
            ["git", "-C", str(ROOT), "archive", required_revision],
            stdout=subprocess.PIPE,
        )
        subprocess.run(
            ["tar", "-x", "-C", str(reconstructed)],
            stdin=archive.stdout,
            check=True,
        )
        assert archive.stdout is not None
        archive.stdout.close()
        assert archive.wait() == 0
        subprocess.run(
            ["git", "apply", "--check", str(patch)], cwd=reconstructed, check=True
        )
        subprocess.run(["git", "apply", str(patch)], cwd=reconstructed, check=True)
        wrapper = reconstructed / "scripts/jit-rustc-workspace-wrapper.sh"
        assert sha256(wrapper) == "891990fa6e76342823b0e28b9ee1f6d84256efe1a4b58deda8388d791589fc48"
        assert all((reconstructed / path).exists() for path in EXPECTED_PATCH_PATHS)
        for executable in [
            "scripts/jit-rustc-workspace-wrapper-selftest.sh",
            "scripts/jit-rustc-workspace-wrapper.sh",
            "scripts/stage-jit-rustc-workspace-wrapper.sh",
        ]:
            assert os.access(reconstructed / executable, os.X_OK)
        selftest = subprocess.run(
            [str(reconstructed / "scripts/jit-rustc-workspace-wrapper-selftest.sh")],
            cwd=reconstructed,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        assert selftest.returncode == 0, selftest.stderr
        assert "SELFTEST: all assertions passed" in selftest.stdout
        assert "ordinary CLI main passes through byte-for-byte" in selftest.stdout
        assert "mod jit_rustc_workspace_wrapper_tests;" in (
            reconstructed / "crates/jit/tests/scratch_build/main.rs"
        ).read_text(encoding="utf-8")


def validate() -> None:
    summary = load(HERE / "summary.json")
    audit = load(HERE / "compiler-audit.json")
    baseline = load(HERE / "screen/opt0-build/baseline.json")
    candidate = load(HERE / "screen/candidate-build/baseline.json")

    required_revision = "d7382ef4c66daa6f117f08422100d10969200965"
    assert summary["source"]["revision"] == required_revision
    assert summary["source"]["required_revision"] == required_revision
    assert sha256(HERE / "rejected-candidate.patch") == summary["source"][
        "candidate_patch_sha256"
    ]
    validate_rejected_patch(required_revision)
    assert summary["decision"] == {
        "candidate_accepted": False,
        "integration_retained": False,
        "reason": "The candidate representative library rebuild took 48.851s, exceeding the matched 48.414s ceiling (26.129% regression versus the 25% maximum). REQ-03 therefore requires rejection before runtime sampling.",
        "thresholds_weakened": False,
        "value": "no_change",
    }

    baseline_clean = baseline["medians"]["clean_test_no_run_wall_seconds"]
    candidate_clean = candidate["medians"]["clean_test_no_run_wall_seconds"]
    baseline_rebuild = baseline["medians"]["rebuild_wall_seconds"]
    candidate_rebuild = candidate["medians"]["rebuild_wall_seconds"]
    assert baseline_clean == summary["build_screen"]["baseline_clean_test_build_seconds"]
    assert candidate_clean == summary["build_screen"]["candidate_clean_test_build_seconds"]
    assert baseline_rebuild == summary["build_screen"]["baseline_representative_rebuild_seconds"]
    assert candidate_rebuild == summary["build_screen"]["candidate_representative_rebuild_seconds"]
    assert candidate_clean <= baseline_clean * 1.25
    assert candidate_rebuild > baseline_rebuild * 1.25
    assert candidate["rebuild_samples"][0]["probe_restored_verified"] is True
    assert summary["acceptance_only_runtime"] == {
        "cargo_ci_runs": 0,
        "doctest_samples": 0,
        "nextest_samples": 0,
        "status": "not_run_after_hard_rebuild_rejection",
    }

    baseline_ids, baseline_digest = inventory_identities(
        HERE / "screen/opt0-build/pre-change-test-inventory.json"
    )
    candidate_ids, candidate_digest = inventory_identities(
        HERE / "screen/candidate-build/pre-change-test-inventory.json"
    )
    assert baseline_ids == candidate_ids
    assert baseline_digest == candidate_digest == summary["identity"]["normalized_sha256"]
    regular = [identity for identity in baseline_ids if identity[1] != "doctest"]
    doctests = [identity for identity in baseline_ids if identity[1] == "doctest"]
    assert sum(not identity[3] for identity in regular) == summary["identity"]["regular_runnable"]
    assert sum(identity[3] for identity in regular) == summary["identity"]["regular_ignored"]
    assert len(doctests) == summary["identity"]["doctest_count"]
    assert sum(identity[3] for identity in doctests) == summary["identity"]["doctest_ignored"]

    assert audit["authority"] == "one fresh cargo test --workspace --no-run command"
    assert audit["record_count"] == len(audit["records"]) == 25
    optimized = [record for record in audit["records"] if record["decision"] == "optimize"]
    passthrough = [record for record in audit["records"] if record["decision"] == "passthrough"]
    assert len(optimized) == audit["optimized_count"] == 1
    assert len(passthrough) == audit["passthrough_count"] == 24
    assert audit["rejected_count"] == audit["conflicting_opt_level_count"] == 0
    assert optimized[0]["root"] == str((ROOT / "crates/jit/src/lib.rs").resolve())
    assert optimized[0]["package"] == optimized[0]["crate"] == "jit"
    assert optimized[0]["has_test"] == optimized[0]["opt_level_count"] == "0"
    ordinary_main = [
        record
        for record in passthrough
        if record["root"] == str((ROOT / "crates/jit/src/main.rs").resolve())
        and record["package"] == record["crate"] == "jit"
        and record["has_test"] == "0"
    ]
    assert len(ordinary_main) == 1
    assert all(record["decision"] == "passthrough" for record in passthrough)

    candidate_screen = candidate["clean_samples"][0]
    assert candidate_screen["inventory"]["unique_active_test_executable_bytes"] == summary[
        "size_screen"
    ]["candidate_active_test_executable_bytes"]
    assert candidate_screen["target_dir_bytes"] == summary["size_screen"][
        "candidate_target_dir_bytes"
    ]
    assert summary["size_screen"]["passed"] is True

    line_table = (HERE / "raw/line-table.txt").read_text(encoding="utf-8")
    backtrace = (HERE / "raw/backtrace.stderr").read_text(encoding="utf-8")
    selftest = (HERE / "raw/wrapper-selftest.stdout").read_text(encoding="utf-8")
    assert ".debug_line" in line_table
    assert "main.rs                                 2214" in line_table
    assert "lib.rs" in line_table
    assert "at ./crates/jit/src/main.rs:2214:9" in backtrace
    assert int((HERE / "raw/backtrace.exit-code").read_text()) == 101
    assert "SELFTEST: all assertions passed" in selftest

    # A hard reject must leave the measured integration absent and every
    # modified tracked source byte-identical to the fixed revision.
    for rejected_path in [
        ROOT / "scripts/jit-rustc-workspace-wrapper.sh",
        ROOT / "scripts/stage-jit-rustc-workspace-wrapper.sh",
        ROOT / "scripts/jit-rustc-workspace-wrapper-selftest.sh",
        ROOT / "crates/jit/tests/scratch_build/jit_rustc_workspace_wrapper_tests.rs",
    ]:
        assert not rejected_path.exists(), rejected_path
    assert "RUSTC_WORKSPACE_WRAPPER" not in (ROOT / "scripts/cargo-ci.sh").read_text(
        encoding="utf-8"
    )
    for restored_path in [
        "crates/jit/tests/scratch_build/build_profile_policy_tests.rs",
        "crates/jit/tests/scratch_build/main.rs",
        "scripts/benchmark-rust-build.sh",
        "scripts/cargo-ci.sh",
    ]:
        base_bytes = subprocess.run(
            ["git", "-C", str(ROOT), "show", f"{required_revision}:{restored_path}"],
            check=True,
            stdout=subprocess.PIPE,
        ).stdout
        assert (ROOT / restored_path).read_bytes() == base_bytes, restored_path


def write_manifest() -> None:
    files = sorted(
        path for path in HERE.rglob("*") if path.is_file() and path.name != "SHA256SUMS"
    )
    lines = [f"{sha256(path)}  {path.relative_to(HERE).as_posix()}" for path in files]
    destination = HERE / "SHA256SUMS"
    temporary = destination.with_name(f".SHA256SUMS.tmp.{os.getpid()}")
    temporary.write_text("\n".join(lines) + "\n", encoding="utf-8")
    temporary.replace(destination)


def validate_manifest() -> None:
    manifest = HERE / "SHA256SUMS"
    assert manifest.is_file()
    for line in manifest.read_text(encoding="utf-8").splitlines():
        expected, relative = line.split("  ", 1)
        assert relative != "SHA256SUMS"
        assert sha256(HERE / relative) == expected, relative


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write-manifest", action="store_true")
    args = parser.parse_args()
    validate()
    if args.write_manifest:
        write_manifest()
    validate_manifest()
    print("jit-library-artifact-wrapper-b883f916: evidence PASS")


if __name__ == "__main__":
    main()
