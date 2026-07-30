#!/usr/bin/env python3
"""Verify that the release metadata of every shipped component agrees.

One product release version has to reach the CLI, the server, the MCP package,
the web bundle, their lock metadata, the release tag, and the compatibility
record. The same run checks the release's legal and narrative metadata: the
license texts the manifest expression names, the changelog entry for the
declared version, the compatibility-and-upgrade record, and the committed
release-note source the publication workflow renders.
"""

from __future__ import annotations

import argparse
import json
import os.path
from pathlib import Path
import re
import sys
import tomllib
from typing import Any, Callable, NamedTuple


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
# The compatibility record is also the upgrade record: this heading is what
# makes the upgrade expectations observable to the check.
REQUIRED_COMPATIBILITY_SECTIONS = ("## Upgrade expectations",)

CHANGELOG_PATH = Path("CHANGELOG.md")
# A released Keep a Changelog entry: a semantic version and its release date.
# `## [Unreleased]` deliberately does not match.
CHANGELOG_RELEASE_PATTERN = re.compile(
    r"^## \[(\d+\.\d+\.\d+)\] - \d{4}-\d{2}-\d{2}\s*$", re.MULTILINE
)

RELEASE_NOTES_ROOT = Path("docs/release-notes")
MARKDOWN_LINK_PATTERN = re.compile(r"\]\(([^)\s]+)\)")
FENCED_BLOCK_PATTERN = re.compile(r"^```", re.MULTILINE)

# The workspace manifest owns the SPDX expression; the published npm package
# restates it and must agree.
LICENSE_EXPRESSION_MANIFEST = Path("Cargo.toml")
PUBLISHED_PACKAGE_MANIFEST = Path("mcp-server/package.json")
LICENSE_OPERATORS = frozenset({"OR", "AND", "WITH"})
FILLED_COPYRIGHT_PATTERN = re.compile(
    r"^Copyright \(c\) \d{4} [^<\n]+$", re.MULTILINE
)


class LicenseContract(NamedTuple):
    """Completeness contract for one SPDX license identifier.

    `phrases` are structural anchors spanning the whole text, so a truncated or
    paraphrased copy fails. `requires_filled_copyright` marks the licenses whose
    text carries a project-specific copyright line rather than a placeholder the
    upstream text keeps verbatim.
    """

    phrases: tuple[str, ...]
    requires_filled_copyright: bool


LICENSE_CONTRACTS: dict[str, LicenseContract] = {
    "MIT": LicenseContract(
        phrases=(
            "Permission is hereby granted, free of charge",
            "The above copyright notice and this permission notice shall be included",
            'THE SOFTWARE IS PROVIDED "AS IS"',
        ),
        requires_filled_copyright=True,
    ),
    "Apache-2.0": LicenseContract(
        phrases=(
            "Apache License",
            "Version 2.0, January 2004",
            "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",
            "END OF TERMS AND CONDITIONS",
            "APPENDIX: How to apply the Apache License to your work.",
        ),
        requires_filled_copyright=False,
    ),
}


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


def license_identifiers(expression: str) -> tuple[str, ...]:
    """Return the SPDX identifiers an expression names, in declaration order."""
    return tuple(
        dict.fromkeys(
            token
            for token in re.split(r"[()\s]+", expression)
            if token and token.upper() not in LICENSE_OPERATORS
        )
    )


def license_text_path(identifier: str) -> Path:
    """Return the repository-root license file an SPDX identifier is carried in."""
    return Path(f"LICENSE-{identifier.split('-')[0].upper()}")


def verify_license_texts(root: Path, findings: list[str]) -> tuple[Path, ...]:
    """Check the license texts the manifest expression names; return their paths."""
    try:
        expression = nested_string(
            read_toml(root / LICENSE_EXPRESSION_MANIFEST),
            "workspace",
            "package",
            "license",
        )
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        findings.append(
            f"license expression at {LICENSE_EXPRESSION_MANIFEST}: {error}"
        )
        return ()

    try:
        published = nested_string(
            read_json(root / PUBLISHED_PACKAGE_MANIFEST), "license"
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        findings.append(f"license expression at {PUBLISHED_PACKAGE_MANIFEST}: {error}")
    else:
        if published != expression:
            findings.append(
                f"license expression {published!r} at {PUBLISHED_PACKAGE_MANIFEST} "
                f"differs from {expression!r} at {LICENSE_EXPRESSION_MANIFEST}"
            )

    carried: list[Path] = []
    for identifier in license_identifiers(expression):
        contract = LICENSE_CONTRACTS.get(identifier)
        if contract is None:
            findings.append(
                f"license identifier {identifier!r} in {expression!r} has no "
                "completeness contract; declare its required text before shipping it"
            )
            continue

        relative = license_text_path(identifier)
        carried.append(relative)
        try:
            text = (root / relative).read_text(encoding="utf-8")
        except OSError as error:
            findings.append(f"license text at {relative}: {error}")
            continue

        findings.extend(
            f"license text at {relative} is missing required text {phrase!r}"
            for phrase in contract.phrases
            if phrase not in text
        )
        if contract.requires_filled_copyright and not FILLED_COPYRIGHT_PATTERN.search(
            text
        ):
            findings.append(
                f"license text at {relative} has no filled copyright line "
                "(expected 'Copyright (c) <year> <holder>')"
            )

    return tuple(carried)


def verify_changelog(root: Path, expected: str, findings: list[str]) -> None:
    """Check that the changelog's newest released entry is the declared version."""
    try:
        text = (root / CHANGELOG_PATH).read_text(encoding="utf-8")
    except OSError as error:
        findings.append(f"changelog at {CHANGELOG_PATH}: {error}")
        return

    releases = CHANGELOG_RELEASE_PATTERN.findall(text)
    if not releases:
        findings.append(
            f"changelog at {CHANGELOG_PATH} records no released version entry; "
            f"expected a '## [{expected}] - YYYY-MM-DD' heading"
        )
    elif releases[0] != expected:
        findings.append(
            f"changelog at {CHANGELOG_PATH} records {releases[0]} as its newest "
            f"release, expected {expected}"
        )


def linked_paths(document: Path, text: str) -> frozenset[str]:
    """Return every intra-repository markdown link target, root-relative."""
    return frozenset(
        os.path.normpath(str(document.parent / target.split("#", 1)[0]))
        for target in MARKDOWN_LINK_PATTERN.findall(text)
        if "://" not in target and not target.startswith("#")
    )


def verify_release_note(
    root: Path,
    expected: str,
    license_paths: tuple[Path, ...],
    findings: list[str],
) -> None:
    """Check the committed release-note source the publication workflow renders."""
    relative = RELEASE_NOTES_ROOT / f"v{expected}.md"
    try:
        text = (root / relative).read_text(encoding="utf-8")
    except OSError as error:
        findings.append(f"release note at {relative}: {error}")
        return

    heading = f"# JIT v{expected}"
    if not text.startswith(f"{heading}\n"):
        findings.append(f"release note at {relative} does not open with {heading!r}")

    if FENCED_BLOCK_PATTERN.search(text):
        findings.append(
            f"release note at {relative} carries a fenced command block; "
            "installation commands belong in the canonical installation guide"
        )

    targets = linked_paths(relative, text)
    findings.extend(
        f"release note at {relative} does not link shipped license asset {asset}"
        for asset in license_paths
        if str(asset) not in targets
    )
    if str(COMPATIBILITY_PATH) not in targets:
        findings.append(f"release note at {relative} does not cite {COMPATIBILITY_PATH}")


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
        findings.extend(
            f"compatibility declaration at {COMPATIBILITY_PATH} is missing required "
            f"section {section!r}"
            for section in REQUIRED_COMPATIBILITY_SECTIONS
            if section not in compatibility_text
        )

    license_paths = verify_license_texts(root, findings)

    if expected is not None:
        verify_changelog(root, expected, findings)
        verify_release_note(root, expected, license_paths, findings)

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
        f"product version {expected}; manifests, locks, tag, compatibility and "
        "upgrade record, license texts, changelog entry, and release-note source "
        "agree"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
