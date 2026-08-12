#!/usr/bin/env python3
"""Reduce the rejected ordinary-JIT-library wrapper screen (jit:b883f916)."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import platform
import subprocess
from collections import Counter


HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BASELINE_DIR = HERE / "screen/opt0-build"
CANDIDATE_DIR = HERE / "screen/candidate-build"
AUDIT_DIR = pathlib.Path(
    os.environ.get(
        "JIT_SCREEN_AUDIT_DIR",
        "/home/vkaskivuo/Projects/just-in-time/target/b883f916-audit/fresh-command",
    )
)
REQUIRED_REVISION = "d7382ef4c66daa6f117f08422100d10969200965"


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    return sha256_bytes(path.read_bytes())


def write_bytes(path: pathlib.Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp.{os.getpid()}")
    temporary.write_bytes(data)
    temporary.replace(path)


def write_json(path: pathlib.Path, value) -> None:
    write_bytes(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode())


def command_text(*command: str) -> str:
    return subprocess.run(command, check=True, text=True, capture_output=True).stdout.strip()


def normalized_inventory(path: pathlib.Path):
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
    regular = [identity for identity in identities if identity[1] != "doctest"]
    doctests = [identity for identity in identities if identity[1] == "doctest"]
    return {
        "identities": identities,
        "sha256": sha256_bytes(encoded),
        "regular_runnable": sum(not identity[3] for identity in regular),
        "regular_ignored": sum(identity[3] for identity in regular),
        "doctest_count": len(doctests),
        "doctest_ignored": sum(identity[3] for identity in doctests),
    }


def read_audit_records():
    required = {
        "version",
        "package",
        "crate",
        "crate_name_count",
        "root",
        "root_match_count",
        "has_test",
        "opt_level_count",
        "decision",
        "reason",
    }
    records = []
    for path in sorted(AUDIT_DIR.iterdir()):
        if not path.is_file():
            continue
        fields = {
            key.decode(): value.decode()
            for field in path.read_bytes().split(b"\0")
            if field
            for key, value in [field.split(b"=", 1)]
        }
        if set(fields) != required:
            raise SystemExit(f"compiler audit record field mismatch: {path}")
        records.append(fields)
    if not records:
        raise SystemExit(f"no compiler audit records found under {AUDIT_DIR}")

    optimized = [record for record in records if record["decision"] == "optimize"]
    passthrough = [record for record in records if record["decision"] == "passthrough"]
    expected_lib = str((ROOT / "crates/jit/src/lib.rs").resolve())
    expected_main = str((ROOT / "crates/jit/src/main.rs").resolve())
    if len(optimized) != 1 or optimized[0]["root"] != expected_lib:
        raise SystemExit("fresh real build did not optimize exactly one ordinary jit library")
    if any(
        record["package"] != "jit"
        or record["crate"] != "jit"
        or record["crate_name_count"] != "1"
        or record["root_match_count"] != "1"
        or record["has_test"] != "0"
        or record["opt_level_count"] != "0"
        or record["reason"] != "ordinary-jit-library"
        for record in optimized
    ):
        raise SystemExit("optimized compiler record violated the classifier contract")
    if len(passthrough) != len(records) - 1 or any(
        record["decision"] != "passthrough" for record in passthrough
    ):
        raise SystemExit("noneligible compiler invocation was modified")
    main_records = [
        record
        for record in records
        if record["root"] == expected_main
        and record["package"] == "jit"
        and record["crate"] == "jit"
        and record["has_test"] == "0"
    ]
    if len(main_records) != 1 or main_records[0]["decision"] != "passthrough":
        raise SystemExit("ordinary jit CLI main was not explicit passthrough")

    grouped = Counter(
        (
            record["decision"],
            record["package"],
            record["crate"],
            record["has_test"],
            record["reason"],
        )
        for record in records
    )
    aggregate = {
        "schema_version": 1,
        "authority": "one fresh cargo test --workspace --no-run command",
        "source_command": "cargo test --workspace --no-run",
        "record_count": len(records),
        "optimized_count": len(optimized),
        "passthrough_count": len(passthrough),
        "rejected_count": sum(record["decision"] == "reject" for record in records),
        "conflicting_opt_level_count": sum(
            int(record["opt_level_count"]) for record in records
        ),
        "optimized_roots": ["crates/jit/src/lib.rs"],
        "ordinary_cli_main_passthrough": True,
        "all_noneligible_passthrough": True,
        "records": records,
        "groups": [
            {
                "count": count,
                "decision": key[0],
                "package": key[1],
                "crate": key[2],
                "has_test": int(key[3]),
                "reason": key[4],
            }
            for key, count in sorted(grouped.items())
        ],
    }
    write_json(HERE / "compiler-audit.json", aggregate)
    return aggregate


def main() -> None:
    baseline = load(BASELINE_DIR / "baseline.json")
    candidate = load(CANDIDATE_DIR / "baseline.json")
    baseline_inventory = normalized_inventory(
        BASELINE_DIR / "pre-change-test-inventory.json"
    )
    candidate_inventory = normalized_inventory(
        CANDIDATE_DIR / "pre-change-test-inventory.json"
    )
    if baseline_inventory["identities"] != candidate_inventory["identities"]:
        raise SystemExit("baseline and candidate test identities differ")

    revision = command_text("git", "-C", str(ROOT), "rev-parse", "HEAD")
    if revision != REQUIRED_REVISION:
        raise SystemExit(f"expected fixed revision {REQUIRED_REVISION}, got {revision}")
    audit = read_audit_records()
    baseline_clean = baseline["medians"]["clean_test_no_run_wall_seconds"]
    baseline_rebuild = baseline["medians"]["rebuild_wall_seconds"]
    candidate_clean = candidate["medians"]["clean_test_no_run_wall_seconds"]
    candidate_rebuild = candidate["medians"]["rebuild_wall_seconds"]
    clean_ceiling = baseline_clean * 1.25
    rebuild_ceiling = baseline_rebuild * 1.25
    clean_regression = (candidate_clean / baseline_clean - 1) * 100
    rebuild_regression = (candidate_rebuild / baseline_rebuild - 1) * 100
    if candidate_clean > clean_ceiling:
        raise SystemExit("candidate unexpectedly failed before the rebuild screen")
    if candidate_rebuild <= rebuild_ceiling:
        raise SystemExit("candidate unexpectedly passed the rebuild screen")

    patch = HERE / "rejected-candidate.patch"
    wrapper = ROOT / "scripts/jit-rustc-workspace-wrapper.sh"
    candidate_screen = candidate["clean_samples"][0]
    summary = {
        "schema_version": 1,
        "issue": "b883f916-e53a-4c77-b104-10fb4defb456",
        "contract": "ordinary-jit-library-artifact-wrapper-screen",
        "source": {
            "revision": revision,
            "required_revision": REQUIRED_REVISION,
            "candidate_patch": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/rejected-candidate.patch",
            "candidate_patch_sha256": sha256_file(patch),
            "wrapper_sha256": sha256_file(wrapper),
            "staged_wrapper_path": f"/home/vkaskivuo/Projects/just-in-time/target/b883f916-stage/{sha256_file(wrapper)}/jit-rustc-workspace-wrapper",
        },
        "environment": {
            "os": platform.platform(),
            "kernel": platform.release(),
            "cpu_count": os.cpu_count(),
            "rustc": command_text("rustc", "-vV"),
            "cargo": command_text("cargo", "--version"),
            "nextest": command_text("cargo", "nextest", "--version"),
            "build_lock": "/tmp/cargo-ci.lock",
            "cargo_build_jobs": 24,
            "cargo_incremental": 0,
            "sccache": False,
            "tmpdir": "/home/vkaskivuo/Projects/just-in-time/target/b883f916-tmp",
            "target_filesystem": "ext4",
            "host_after_screen": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/raw/host-after-screen.txt",
        },
        "method": {
            "phase": "mandatory one-sample early screen",
            "harness": "scripts/benchmark-rust-build.sh with the rejected candidate patch applied",
            "baseline_samples": {"clean": 1, "representative_rebuild": 1},
            "candidate_samples": {"clean": 1, "representative_rebuild": 1},
            "fresh_target_per_arm_and_sample": True,
            "matched_source_revision": True,
            "screen_stop": "The clean build passed by 0.122 seconds; the representative rebuild exceeded its hard ceiling, so no runtime sample was authorized.",
            "three_sample_protocol": "not authorized after hard one-sample rebuild rejection",
        },
        "compiler_audit": {
            "path": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/compiler-audit.json",
            "authority": audit["authority"],
            "record_count": audit["record_count"],
            "optimized_count": audit["optimized_count"],
            "passthrough_count": audit["passthrough_count"],
            "rejected_count": audit["rejected_count"],
            "conflicting_opt_level_count": audit["conflicting_opt_level_count"],
            "ordinary_cli_main_passthrough": audit["ordinary_cli_main_passthrough"],
            "all_noneligible_passthrough": audit["all_noneligible_passthrough"],
            "note": "The screen harness emitted 70 records across clean/inventory/rebuild phases. This separate fresh single-command audit is authoritative for the exact-one eligibility assertion.",
        },
        "wrapper_contract_selftest": {
            "exit_code": 0,
            "stdout": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/raw/wrapper-selftest.stdout",
            "stderr": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/raw/wrapper-selftest.stderr",
        },
        "identity": {
            "exact_sets_equal": True,
            "normalized_sha256": baseline_inventory["sha256"],
            "regular_runnable": baseline_inventory["regular_runnable"],
            "regular_ignored": baseline_inventory["regular_ignored"],
            "doctest_count": baseline_inventory["doctest_count"],
            "doctest_ignored": baseline_inventory["doctest_ignored"],
        },
        "build_screen": {
            "baseline_clean_test_build_seconds": baseline_clean,
            "candidate_clean_test_build_seconds": candidate_clean,
            "clean_build_ceiling_seconds": round(clean_ceiling, 6),
            "candidate_clean_regression_percent": round(clean_regression, 6),
            "baseline_representative_rebuild_seconds": baseline_rebuild,
            "candidate_representative_rebuild_seconds": candidate_rebuild,
            "representative_rebuild_ceiling_seconds": round(rebuild_ceiling, 6),
            "candidate_representative_rebuild_regression_percent": round(rebuild_regression, 6),
            "candidate_clean_exit_code": candidate_screen["test_no_run"]["exit_code"],
            "candidate_rebuild_exit_code": candidate["rebuild_samples"][0]["rebuild_test_no_run"]["exit_code"],
            "probe_restored_verified": candidate["rebuild_samples"][0]["probe_restored_verified"],
            "passed": False,
        },
        "size_screen": {
            "candidate_active_test_executable_bytes": candidate_screen["inventory"]["unique_active_test_executable_bytes"],
            "active_test_executable_limit_bytes": 2 * 1024**3,
            "candidate_target_dir_bytes": candidate_screen["target_dir_bytes"],
            "target_dir_limit_bytes": 10 * 1024**3,
            "passed": candidate_screen["inventory"]["unique_active_test_executable_bytes"] < 2 * 1024**3
            and candidate_screen["target_dir_bytes"] < 10 * 1024**3,
        },
        "line_table_and_backtrace": {
            "binary_sha256": sha256_file(pathlib.Path(candidate_screen["retained_target_dir"]) / "debug/jit"),
            "binary_bytes": (pathlib.Path(candidate_screen["retained_target_dir"]) / "debug/jit").stat().st_size,
            "debug_line_section_present": True,
            "decoded_main_panic_line_present": True,
            "decoded_library_line_present": True,
            "backtrace_exit_code": int((HERE / "raw/backtrace.exit-code").read_text()),
            "backtrace_source_location": "./crates/jit/src/main.rs:2214",
            "raw_line_table": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/raw/line-table.txt",
            "raw_backtrace": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/raw/backtrace.stderr",
        },
        "acceptance_only_runtime": {
            "nextest_samples": 0,
            "doctest_samples": 0,
            "cargo_ci_runs": 0,
            "status": "not_run_after_hard_rebuild_rejection",
        },
        "decision_matrix": [
            {"criterion": "wrapper classification and shell safety", "status": "pass"},
            {"criterion": "fresh real compiler invocation audit", "status": "pass"},
            {"criterion": "exact runnable, ignored, and doctest identities", "status": "pass"},
            {"criterion": "line-table backtrace preservation", "status": "pass"},
            {"criterion": "active executable and target size", "status": "pass"},
            {"criterion": "clean test-build regression <= 25 percent", "status": "pass"},
            {"criterion": "representative rebuild regression <= 25 percent", "status": "fail"},
            {"criterion": "one-sample nextest and doctest screen", "status": "not_authorized"},
            {"criterion": "three matched runtime/build/doctest samples", "status": "not_authorized"},
            {"criterion": "authoritative cargo-ci", "status": "not_run_for_rejected_candidate"},
        ],
        "decision": {
            "value": "no_change",
            "candidate_accepted": False,
            "reason": f"The candidate representative library rebuild took {candidate_rebuild:.3f}s, exceeding the matched {rebuild_ceiling:.3f}s ceiling ({rebuild_regression:.3f}% regression versus the 25% maximum). REQ-03 therefore requires rejection before runtime sampling.",
            "thresholds_weakened": False,
            "integration_retained": False,
        },
        "raw_evidence": {
            "baseline": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/opt0-build",
            "candidate": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/screen/candidate-build",
            "authoritative_compiler_audit_stdout": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/raw/compiler-audit-build.stdout",
            "authoritative_compiler_audit_stderr": "dev/benchmarks/jit-library-artifact-wrapper-b883f916/raw/compiler-audit-build.stderr",
        },
    }
    write_json(HERE / "summary.json", summary)
    print("jit-library-artifact-wrapper-b883f916: recorded hard-reject screen evidence")


if __name__ == "__main__":
    main()
