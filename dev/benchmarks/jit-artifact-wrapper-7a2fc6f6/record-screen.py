#!/usr/bin/env python3
"""Record the rejected ordinary-jit compiler-wrapper screen (jit:7a2fc6f6)."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import platform
import subprocess
import sys
from collections import Counter


HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SCREEN = HERE / "screen"
BASELINE_DIR = SCREEN / "opt0-build"
CANDIDATE_DIR = SCREEN / "candidate-build"
AUDIT_DIR = pathlib.Path(
    os.environ.get(
        "JIT_SCREEN_AUDIT_DIR",
        "/home/vkaskivuo/.cache/jit-7a2fc6f6/audit/screen-candidate",
    )
)


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def load_jsonl_one(path: pathlib.Path):
    lines = [line for line in path.read_text(encoding="utf-8").splitlines() if line]
    if len(lines) != 1:
        raise SystemExit(f"expected exactly one record in {path}, got {len(lines)}")
    return json.loads(lines[0])


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
    identities = []
    for target in inventory["targets"]:
        kind = ",".join(target["kind"])
        for test in target["tests"]:
            identities.append(
                (target["name"], kind, test["name"], bool(test["ignored"]))
            )
    identities.sort()
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
    records = []
    for path in sorted(AUDIT_DIR.iterdir()):
        if not path.is_file():
            continue
        fields = {}
        for field in path.read_bytes().split(b"\0"):
            if not field:
                continue
            key, value = field.split(b"=", 1)
            fields[key.decode()] = value.decode()
        records.append(fields)
    if not records:
        raise SystemExit(f"no compiler audit records found under {AUDIT_DIR}")

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
    if any(set(record) != required for record in records):
        raise SystemExit("compiler audit record field mismatch")
    if any(record["decision"] == "reject" for record in records):
        raise SystemExit("real candidate contained a rejected compiler invocation")

    optimized = [record for record in records if record["decision"] == "optimize"]
    expected_roots = {
        str((ROOT / "crates/jit/src/lib.rs").resolve()),
        str((ROOT / "crates/jit/src/main.rs").resolve()),
    }
    if {record["root"] for record in optimized} != expected_roots or len(optimized) != 2:
        raise SystemExit("real candidate did not optimize exactly ordinary jit lib/main")
    if any(
        record["package"] != "jit"
        or record["crate"] != "jit"
        or record["crate_name_count"] != "1"
        or record["root_match_count"] != "1"
        or record["has_test"] != "0"
        or record["opt_level_count"] != "0"
        for record in optimized
    ):
        raise SystemExit("optimized compiler record violated the classifier contract")
    if any(
        record["decision"] != "passthrough"
        for record in records
        if record not in optimized
    ):
        raise SystemExit("non-eligible workspace compiler invocation was modified")

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
        "source_command": "fresh candidate cargo test --workspace --no-run",
        "record_count": len(records),
        "optimized_count": len(optimized),
        "passthrough_count": len(records) - len(optimized),
        "conflicting_opt_level_count": sum(
            int(record["opt_level_count"]) for record in records
        ),
        "optimized_roots": sorted(expected_roots),
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


def record_line_tables_and_backtrace(candidate_binary: pathlib.Path):
    sections = command_text("readelf", "-S", str(candidate_binary))
    if ".debug_line" not in sections:
        raise SystemExit("candidate binary has no .debug_line section")
    decoded = command_text("readelf", "--debug-dump=decodedline", str(candidate_binary))
    decoded_evidence = [
        line
        for line in decoded.splitlines()
        if "crates/jit/src/main.rs" in line
        or "crates/jit/src/lib.rs" in line
        or ("main.rs" in line and "2214" in line)
    ]
    if not any("main.rs" in line and "2214" in line for line in decoded_evidence):
        raise SystemExit("candidate line table does not resolve main.rs:2214")
    write_bytes(
        HERE / "raw/line-table.txt",
        (
            "# readelf -S (debug sections)\n"
            + "\n".join(line for line in sections.splitlines() if ".debug_" in line)
            + "\n# decoded entries for ordinary jit roots and panic site\n"
            + "\n".join(decoded_evidence)
            + "\n"
        ).encode(),
    )

    with open("/dev/full", "wb") as full:
        backtrace = subprocess.run(
            [str(candidate_binary), "--schema"],
            stdout=full,
            stderr=subprocess.PIPE,
            env={**os.environ, "RUST_BACKTRACE": "1"},
        )
    write_bytes(HERE / "raw/backtrace.stderr", backtrace.stderr)
    decoded_backtrace = backtrace.stderr.decode(errors="replace")
    if backtrace.returncode != 101 or "at ./crates/jit/src/main.rs:2214:" not in decoded_backtrace:
        raise SystemExit("candidate did not produce a source-resolved line-table backtrace")
    return {
        "binary_sha256": sha256_file(candidate_binary),
        "binary_bytes": candidate_binary.stat().st_size,
        "debug_line_section_present": True,
        "decoded_main_panic_line_present": True,
        "backtrace_exit_code": backtrace.returncode,
        "backtrace_source_location": "./crates/jit/src/main.rs:2214",
        "raw_line_table": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/raw/line-table.txt",
        "raw_backtrace": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/raw/backtrace.stderr",
    }


def record_wrapper_selftest():
    selftest = ROOT / "scripts/jit-rustc-workspace-wrapper-selftest.sh"
    result = subprocess.run(
        [str(selftest)], cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE
    )
    write_bytes(HERE / "raw/wrapper-selftest.stdout", result.stdout)
    write_bytes(HERE / "raw/wrapper-selftest.stderr", result.stderr)
    if result.returncode != 0 or b"SELFTEST: all assertions passed" not in result.stdout:
        raise SystemExit("wrapper contract selftest failed while recording evidence")
    return {
        "exit_code": result.returncode,
        "stdout": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/raw/wrapper-selftest.stdout",
        "stderr": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/raw/wrapper-selftest.stderr",
    }


def main() -> None:
    baseline = load(BASELINE_DIR / "baseline.json")
    candidate = load_jsonl_one(CANDIDATE_DIR / "raw/clean-samples.jsonl")
    baseline_inventory = normalized_inventory(
        BASELINE_DIR / "pre-change-test-inventory.json"
    )
    candidate_inventory = normalized_inventory(
        CANDIDATE_DIR / "pre-change-test-inventory.json"
    )
    if baseline_inventory["identities"] != candidate_inventory["identities"]:
        raise SystemExit("baseline and candidate test identities differ")

    audit = read_audit_records()
    candidate_target = pathlib.Path(candidate["retained_target_dir"])
    binary_evidence = record_line_tables_and_backtrace(candidate_target / "debug/jit")
    selftest = record_wrapper_selftest()

    baseline_clean = baseline["medians"]["clean_test_no_run_wall_seconds"]
    baseline_rebuild = baseline["medians"]["rebuild_wall_seconds"]
    candidate_clean = candidate["test_no_run"]["wall_seconds"]
    clean_ceiling = baseline_clean * 1.25
    rebuild_ceiling = baseline_rebuild * 1.25
    clean_regression = (candidate_clean / baseline_clean - 1) * 100
    if candidate_clean <= clean_ceiling:
        raise SystemExit("candidate unexpectedly passed the clean-build screen")

    wrapper = ROOT / "scripts/jit-rustc-workspace-wrapper.sh"
    patch = HERE / "rejected-candidate.patch"
    summary = {
        "schema_version": 1,
        "issue": "7a2fc6f6-854d-4764-8585-3757c6d9c7ac",
        "contract": "ordinary-jit-artifact-wrapper-screen",
        "source": {
            "revision": command_text("git", "-C", str(ROOT), "rev-parse", "HEAD"),
            "required_revision": "31e7d502f92a12b0565a41a4ed37c90860097455",
            "candidate_patch": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/rejected-candidate.patch",
            "candidate_patch_sha256": sha256_file(patch),
            "wrapper_sha256": sha256_file(wrapper),
            "staged_wrapper_path": f"/home/vkaskivuo/.cache/jit-7a2fc6f6/wrappers/{sha256_file(wrapper)}/jit-rustc-workspace-wrapper",
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
            "tmpdir": "/home/vkaskivuo/.cache/jit-7a2fc6f6/tmp",
            "target_filesystem": "ext4",
        },
        "method": {
            "phase": "mandatory one-sample early screen",
            "harness": "scripts/benchmark-rust-build.sh with the rejected candidate patch applied",
            "baseline_samples": {"clean": 1, "representative_rebuild": 1},
            "candidate_samples": {"clean": 1, "representative_rebuild": 0},
            "fresh_target_per_arm_and_sample": True,
            "candidate_stop": "Interrupted with exit 130 during the unmeasured equivalent rebuild setup immediately after the completed clean sample exceeded its hard ceiling; the source probe had not been applied. Full console evidence is retained in raw/candidate-session-interruption.txt.",
            "three_sample_protocol": "not authorized after hard one-sample rejection",
        },
        "compiler_audit": {
            "path": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/compiler-audit.json",
            "record_count": audit["record_count"],
            "optimized_count": audit["optimized_count"],
            "passthrough_count": audit["passthrough_count"],
            "conflicting_opt_level_count": audit["conflicting_opt_level_count"],
            "all_noneligible_passthrough": audit["all_noneligible_passthrough"],
        },
        "wrapper_contract_selftest": selftest,
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
            "representative_rebuild_ceiling_seconds": round(rebuild_ceiling, 6),
            "candidate_representative_rebuild_seconds": None,
            "candidate_clean_exit_code": candidate["test_no_run"]["exit_code"],
            "candidate_clean_max_rss_kb": candidate["test_no_run"]["max_rss_kb"],
            "passed": False,
        },
        "size_screen": {
            "candidate_active_test_executable_bytes": candidate["inventory"]["unique_active_test_executable_bytes"],
            "active_test_executable_limit_bytes": 2 * 1024**3,
            "candidate_target_dir_bytes": candidate["target_dir_bytes"],
            "target_dir_limit_bytes": 10 * 1024**3,
            "passed": candidate["inventory"]["unique_active_test_executable_bytes"] < 2 * 1024**3
            and candidate["target_dir_bytes"] < 10 * 1024**3,
        },
        "line_table_and_backtrace": binary_evidence,
        "acceptance_only_runtime": {
            "nextest_samples": 0,
            "doctest_samples": 0,
            "cargo_ci_runs": 0,
            "status": "not_run_after_hard_clean_build_rejection",
        },
        "decision_matrix": [
            {"criterion": "wrapper classification and shell safety", "status": "pass"},
            {"criterion": "real compiler invocation audit", "status": "pass"},
            {"criterion": "exact runnable, ignored, and doctest identities", "status": "pass"},
            {"criterion": "line-table backtrace preservation", "status": "pass"},
            {"criterion": "active executable and target size", "status": "pass"},
            {"criterion": "clean test-build regression <= 25 percent", "status": "fail"},
            {"criterion": "representative rebuild regression <= 25 percent", "status": "not_run_after_earlier_hard_failure"},
            {"criterion": "three matched runtime/build/doctest samples", "status": "not_authorized"},
            {"criterion": "authoritative cargo-ci", "status": "not_run_for_rejected_candidate"},
        ],
        "decision": {
            "value": "no_change",
            "candidate_accepted": False,
            "reason": f"The first candidate clean test build took {candidate_clean:.3f}s, exceeding the matched {clean_ceiling:.3f}s ceiling ({clean_regression:.2f}% regression versus the 25% maximum). REQ-04 therefore requires rejection before representative rebuild or runtime sampling.",
            "thresholds_weakened": False,
            "integration_retained": False,
        },
        "raw_evidence": {
            "baseline": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/opt0-build",
            "candidate": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/candidate-build",
            "interrupted_post_rejection_session": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/raw/candidate-session-interruption.txt",
            "interrupted_post_rejection_setup_log": "dev/benchmarks/jit-artifact-wrapper-7a2fc6f6/screen/candidate-build/raw/rejected-after-clean/setup-clippy-interrupted.log",
        },
    }
    if summary["source"]["revision"] != summary["source"]["required_revision"]:
        raise SystemExit("screen was not recorded at the required fixed revision")
    write_json(HERE / "summary.json", summary)
    print("jit-artifact-wrapper-7a2fc6f6: recorded hard-reject screen evidence")


if __name__ == "__main__":
    main()
