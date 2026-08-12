#!/usr/bin/env python3
"""Recompute the durable no-change evidence for jit:efaec52f."""

from __future__ import annotations

import hashlib
import json
import pathlib
import subprocess


HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SOURCE_REVISION = "11f096377949c404766a42153ee19e203b54da46"


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    return sha256_bytes(path.read_bytes())


def inventory_identities(path: pathlib.Path, *, doctest: bool):
    inventory = load(path)
    return sorted(
        (
            target["name"],
            ",".join(target["kind"]),
            test["name"],
            bool(test["ignored"]),
        )
        for target in inventory["targets"]
        if (target["kind"] == ["doctest"]) == doctest
        for test in target["tests"]
    )


def canonical_doctest_digest(identities) -> str:
    values = [
        {"target": target, "name": name, "ignored": ignored}
        for target, _kind, name, ignored in identities
    ]
    encoded = (
        json.dumps(values, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
        + "\n"
    ).encode()
    return sha256_bytes(encoded)


def parse_candidate_doctests(all_output: pathlib.Path, ignored_output: pathlib.Path):
    def names(path: pathlib.Path):
        return [
            line.rsplit(": ", 1)[0]
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.endswith((": test", ": benchmark"))
        ]

    all_names = names(all_output)
    ignored_names = set(names(ignored_output))
    assert len(all_names) == len(set(all_names)), "duplicate candidate doctest identity"
    assert ignored_names <= set(all_names)

    def target_for(name: str) -> str:
        if name.startswith("crates/jit/"):
            return "jit"
        if name.startswith("crates/server/"):
            return "jit_server"
        raise AssertionError(f"unknown doctest source root: {name}")

    return sorted(
        (
            target_for(name),
            "doctest",
            name,
            name in ignored_names,
        )
        for name in all_names
    )


def parse_measurement(path: pathlib.Path):
    fields = dict(
        line.split("=", 1)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line
    )
    return {"exit_code": int(fields["exit_code"]), "elapsed_ms": int(fields["elapsed_ms"])}


def compiler_artifacts(path: pathlib.Path):
    return [
        json.loads(line)
        for line in path.read_text(encoding="utf-8").splitlines()
        if line and json.loads(line).get("reason") == "compiler-artifact"
    ]


def classify_package(message) -> str:
    package = message["package_id"]
    if "/crates/jit#" in package:
        return "jit"
    if "/crates/server#" in package:
        return "jit-server"
    return "dependency"


def validate_compiler_scope(summary) -> None:
    scope = summary["compiler_scope"]
    baseline = compiler_artifacts(HERE / "opt0-build-screen/raw/clean-1/cargo-test-list.json")
    candidate = compiler_artifacts(HERE / "scoped-build-screen/raw/clean-1/cargo-test-list.json")
    assert len(baseline) == scope["baseline"]["compiler_artifact_messages"] == 367
    assert len(candidate) == scope["compiler_artifact_messages"] == 367

    for messages, expected in [
        (baseline, {"jit": "0", "jit-server": "0", "dependency": "0"}),
        (candidate, {"jit-server": "0", "dependency": "0"}),
    ]:
        for package, opt_level in expected.items():
            package_messages = [m for m in messages if classify_package(m) == package]
            assert package_messages
            assert {m["profile"]["opt_level"] for m in package_messages} == {opt_level}

    candidate_jit_test = [
        message
        for message in candidate
        if classify_package(message) == "jit" and message["profile"]["test"]
    ]
    assert len(candidate_jit_test) == scope["jit"]["test_profile_artifact_messages"] == 12
    assert {message["profile"]["opt_level"] for message in candidate_jit_test} == {"1"}

    candidate_server_test = [
        message
        for message in candidate
        if classify_package(message) == "jit-server" and message["profile"]["test"]
    ]
    assert len(candidate_server_test) == scope["jit_server"]["test_profile_artifact_messages"] == 3
    assert {message["profile"]["opt_level"] for message in candidate_server_test} == {"0"}


def validate_manifest(summary, provenance) -> None:
    source = summary["source"]
    base = (ROOT / "Cargo.toml").read_bytes()
    assert sha256_bytes(base) == source["restored_manifest_sha256"]
    assert b"[profile.test.package.jit]" not in base
    patch = HERE / "rejected-candidate.patch"
    assert sha256_file(patch) == source["rejected_candidate_sha256"]
    assert source["rejected_candidate_sha256"] == provenance["source"]["rejected_patch_sha256"]
    assert subprocess.run(
        ["git", "apply", "--check", str(patch)], cwd=ROOT, capture_output=True
    ).returncode == 0

    anchor = b'[profile.test]\ndebug = "line-tables-only"\nincremental = true\n\n[workspace.dependencies]'
    replacement = b'[profile.test]\ndebug = "line-tables-only"\nincremental = true\n\n[profile.test.package.jit]\nopt-level = 1\n\n[workspace.dependencies]'
    assert base.count(anchor) == 1
    candidate = base.replace(anchor, replacement)
    assert sha256_bytes(candidate) == source["candidate_manifest_sha256"]
    assert source["candidate_manifest_sha256"] == provenance["source"]["candidate_manifest_sha256"]


def validate_hash_manifest() -> None:
    manifest = HERE / "SHA256SUMS"
    recorded = {}
    for line in manifest.read_text(encoding="utf-8").splitlines():
        digest, name = line.split("  ", 1)
        assert name not in recorded
        recorded[name] = digest
    actual = {
        path.relative_to(HERE).as_posix(): sha256_file(path)
        for path in HERE.rglob("*")
        if path.is_file() and path.name != "SHA256SUMS"
    }
    assert recorded == actual


def validate() -> None:
    summary = load(HERE / "summary.json")
    provenance = load(HERE / "candidate-doctest-recheck/provenance.json")
    baseline_build = load(HERE / "opt0-build-screen/baseline.json")
    candidate_build = load(HERE / "scoped-build-screen/baseline.json")

    assert summary["source"]["baseline_revision"] == SOURCE_REVISION
    assert summary["source"]["candidate_revision"] == SOURCE_REVISION
    assert provenance["source"]["revision"] == SOURCE_REVISION
    validate_manifest(summary, provenance)

    baseline_rebuild = baseline_build["medians"]["rebuild_wall_seconds"]
    candidate_rebuild = candidate_build["medians"]["rebuild_wall_seconds"]
    assert baseline_rebuild == summary["build_screen"]["baseline_rebuild_seconds"] == 35.406
    assert candidate_rebuild == summary["build_screen"]["candidate_rebuild_seconds"] == 64.487
    assert baseline_rebuild * 1.25 == summary["build_screen"]["rebuild_ceiling_seconds"]
    assert candidate_rebuild > baseline_rebuild * 1.25
    assert candidate_build["rebuild_samples"][0]["probe_restored_verified"] is True
    assert summary["decision"]["value"] == "no_change"
    assert summary["decision"]["candidate_accepted"] is False
    assert summary["decision"]["production_profile_change_retained"] is False
    assert summary["thresholds"]["weakened"] is False

    validate_compiler_scope(summary)

    baseline_regular = inventory_identities(
        HERE / "opt0-build-screen/pre-change-test-inventory.json", doctest=False
    )
    candidate_regular = inventory_identities(
        HERE / "scoped-build-screen/pre-change-test-inventory.json", doctest=False
    )
    assert baseline_regular == candidate_regular
    assert len(candidate_regular) == summary["identities"]["candidate_regular_total"] == 4521
    assert sum(not identity[3] for identity in candidate_regular) == 4511
    assert sum(identity[3] for identity in candidate_regular) == 10

    baseline_doctests = inventory_identities(
        HERE / "opt0-build-screen/pre-change-test-inventory.json", doctest=True
    )
    candidate_doctests = parse_candidate_doctests(
        HERE / "candidate-doctest-recheck/raw/cargo-doctest-list.stdout",
        HERE / "candidate-doctest-recheck/raw/cargo-doctest-list-ignored.stdout",
    )
    assert candidate_doctests == baseline_doctests
    assert len(candidate_doctests) == summary["identities"]["doctest"]["candidate_count"] == 63
    assert sum(identity[3] for identity in candidate_doctests) == 0
    digest = canonical_doctest_digest(candidate_doctests)
    assert digest == canonical_doctest_digest(baseline_doctests)
    assert digest == summary["identities"]["doctest"]["candidate_normalized_identity_sha256"]
    assert digest == summary["identities"]["doctest"]["baseline_normalized_identity_sha256"]
    assert digest == provenance["result"]["candidate_normalized_identity_sha256"]
    assert summary["identities"]["doctest"]["exact_sets_equal"] is True

    all_measurement = parse_measurement(
        HERE / "candidate-doctest-recheck/raw/cargo-doctest-list.measurement"
    )
    ignored_measurement = parse_measurement(
        HERE / "candidate-doctest-recheck/raw/cargo-doctest-list-ignored.measurement"
    )
    assert all_measurement == {"exit_code": 0, "elapsed_ms": 33355}
    assert ignored_measurement == {"exit_code": 0, "elapsed_ms": 1633}
    assert provenance["commands"][0]["elapsed_ms"] == all_measurement["elapsed_ms"]
    assert provenance["commands"][1]["elapsed_ms"] == ignored_measurement["elapsed_ms"]
    for name in ["cargo-doctest-list.stderr", "cargo-doctest-list-ignored.stderr"]:
        stderr = (HERE / "candidate-doctest-recheck/raw" / name).read_text(encoding="utf-8")
        assert "Doc-tests jit" in stderr and "Doc-tests jit_server" in stderr
        assert "error:" not in stderr

    failed_attempt = (
        HERE / "scoped-build-screen/raw/clean-1/cargo-doctest-list.log"
    ).read_text(encoding="utf-8")
    assert "No such file or directory" in failed_attempt
    assert summary["decision_matrix"]["doctest_identity_set"] == "pass_exact_candidate_recheck"
    assert summary["method"]["samples"]["candidate_doctest"] == 0
    assert summary["decision_matrix"]["doctest_runtime"] == "not_run_after_authorized_early_reject"

    assert summary["bytes"]["candidate_unique_active_test_executable_bytes"] < 2 * 1024**3
    validate_hash_manifest()


if __name__ == "__main__":
    validate()
    print("test-profile-efaec52f: evidence PASS")
