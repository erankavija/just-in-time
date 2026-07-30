#!/usr/bin/env python3
"""Verify that every shipped component shares one product release version."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import sys
import tomllib
from typing import Any, Callable


COMPATIBILITY_PATH = Path("docs/reference/compatibility.md")
COMPATIBILITY_PATTERN = re.compile(
    r"^\*\*Product compatibility version:\*\* `([^`]+)`$", re.MULTILINE
)
REQUIRED_CAPABILITIES = (
    "native `jit` CLI",
    "`jit-server` Rust server and API",
    "built web UI",
    "`@erankavija/jit-mcp-server` MCP server",
)


def read_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def read_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError("top-level value is not an object")
    return value


def nested_string(value: dict[str, Any], *keys: str) -> str:
    current: Any = value
    for key in keys:
        if not isinstance(current, dict) or key not in current:
            raise ValueError(f"missing {'.'.join(keys)}")
        current = current[key]
    if not isinstance(current, str) or not current:
        raise ValueError(f"{'.'.join(keys)} is not a non-empty string")
    return current


def read_cargo_manifest(path: Path) -> str:
    return nested_string(read_toml(path), "package", "version")


def read_node_manifest(path: Path) -> str:
    return nested_string(read_json(path), "version")


def read_cargo_lock_version(path: Path, package_name: str) -> str:
    packages = read_toml(path).get("package")
    if not isinstance(packages, list):
        raise ValueError("missing package records")
    matches = [
        package
        for package in packages
        if isinstance(package, dict) and package.get("name") == package_name
    ]
    if len(matches) != 1:
        raise ValueError(
            f"expected one {package_name!r} package record, found {len(matches)}"
        )
    return nested_string(matches[0], "version")


def read_node_lock_versions(path: Path) -> tuple[str, str]:
    lock = read_json(path)
    return (
        nested_string(lock, "version"),
        nested_string(lock, "packages", "", "version"),
    )


def load_version(
    root: Path,
    label: str,
    relative: Path,
    reader: Callable[[Path], str],
    findings: list[str],
) -> str | None:
    try:
        return reader(root / relative)
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        findings.append(f"{label} at {relative}: {error}")
        return None


def verify(root: Path, tag: str | None = None) -> tuple[str | None, list[str]]:
    """Return the manifest-derived product version and all contract findings."""
    findings: list[str] = []
    manifest_specs: tuple[tuple[str, Path, Callable[[Path], str]], ...] = (
        ("jit manifest", Path("crates/jit/Cargo.toml"), read_cargo_manifest),
        (
            "jit-server manifest",
            Path("crates/server/Cargo.toml"),
            read_cargo_manifest,
        ),
        (
            "MCP server manifest",
            Path("mcp-server/package.json"),
            read_node_manifest,
        ),
        ("web manifest", Path("web/package.json"), read_node_manifest),
    )
    manifest_versions = {
        label: load_version(root, label, relative, reader, findings)
        for label, relative, reader in manifest_specs
    }
    expected = manifest_versions["jit manifest"]

    if expected is not None:
        findings.extend(
            f"{label} declares {version}, expected {expected} from jit manifest"
            for label, version in manifest_versions.items()
            if version is not None and version != expected
        )

        for package_name, label in (
            ("jit", "jit lock metadata"),
            ("jit-server", "jit-server lock metadata"),
        ):
            locked = load_version(
                root,
                label,
                Path("Cargo.lock"),
                lambda path, name=package_name: read_cargo_lock_version(path, name),
                findings,
            )
            if locked is not None and locked != expected:
                findings.append(f"{label} declares {locked}, expected {expected}")

        for label, relative in (
            ("MCP server lock metadata", Path("mcp-server/package-lock.json")),
            ("web lock metadata", Path("web/package-lock.json")),
        ):
            try:
                lock_versions = read_node_lock_versions(root / relative)
            except (OSError, ValueError, json.JSONDecodeError) as error:
                findings.append(f"{label} at {relative}: {error}")
                continue
            findings.extend(
                f"{label} declares {version}, expected {expected}"
                for version in lock_versions
                if version != expected
            )

        if tag is not None and tag != f"v{expected}":
            findings.append(f"release tag {tag!r} does not match v{expected}")

    nested_lock = root / "crates/jit/Cargo.lock"
    if nested_lock.exists():
        findings.append(
            "non-authoritative nested Rust lockfile crates/jit/Cargo.lock duplicates "
            "the workspace Cargo.lock"
        )

    compatibility = root / COMPATIBILITY_PATH
    try:
        compatibility_text = compatibility.read_text(encoding="utf-8")
    except OSError as error:
        findings.append(f"compatibility declaration at {COMPATIBILITY_PATH}: {error}")
    else:
        declared_versions = COMPATIBILITY_PATTERN.findall(compatibility_text)
        unique_versions = tuple(dict.fromkeys(declared_versions))
        if not declared_versions:
            findings.append(
                f"compatibility declaration at {COMPATIBILITY_PATH} is missing the "
                "product compatibility version"
            )
        elif len(unique_versions) > 1:
            rendered_versions = ", ".join(repr(version) for version in unique_versions)
            findings.append(
                "compatibility declaration has conflicting product compatibility "
                f"versions {rendered_versions}; expected exactly one declaration"
            )
        elif len(declared_versions) > 1:
            findings.append(
                "compatibility declaration has "
                f"{len(declared_versions)} product compatibility version declarations; "
                "expected exactly one"
            )
        elif expected is not None and declared_versions[0] != expected:
            findings.append(
                "compatibility declaration declares "
                f"{declared_versions[0]}, expected {expected}"
            )
        findings.extend(
            f"compatibility declaration is missing supported capability {capability!r}"
            for capability in REQUIRED_CAPABILITIES
            if capability not in compatibility_text
        )

    return expected, findings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.cwd(),
        help="repository root (default: current directory)",
    )
    parser.add_argument(
        "--tag",
        help="release tag to verify; omitted for manifest-only CI checks",
    )
    arguments = parser.parse_args(argv)

    expected, findings = verify(arguments.root.resolve(), arguments.tag)
    if findings:
        for finding in findings:
            print(f"release-version-contract: {finding}", file=sys.stderr)
        return 1

    print(
        "release-version-contract: "
        f"product version {expected}; manifests, locks, tag, compatibility, "
        "and supported capabilities agree"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
