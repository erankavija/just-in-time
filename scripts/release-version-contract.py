#!/usr/bin/env python3
"""Verify that the release metadata of every shipped component agrees.

One product release version has to reach the CLI, the server, the MCP package,
the web bundle, their lock metadata, the release tag, and the compatibility
record, and it has to fall inside the compatible range the embedded profile
package declares. The same run checks the release's legal and narrative
metadata: the license texts the manifest expression names, the changelog entry
for the declared version, the compatibility-and-upgrade record, and the
committed release-note source the publication workflow renders.

With `--declared` the run prints the version it derived instead of its summary,
which is how the publication workflow names the release note it renders without
carrying a version literal of its own. A disagreeing tree still fails, so that
value is only ever printed once every declaration above agrees.
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

# The embedded profile package states which product versions it composes with.
# The binary parses that range for syntax alone, so this is where the range and
# the version it claims to admit are matched.
PROFILE_PACKAGE_MANIFEST = Path("profiles/jit-dogfood/manifest.toml")
# A released version is `major.minor.patch` throughout this contract — the
# changelog heading it checks admits no other form — so a compatibility range is
# read in the same terms. A comparator written as a caret, tilde, wildcard, or
# bare version, or carrying a prerelease or build identifier, is reported rather
# than interpreted, so a range this check cannot match exactly never passes as
# an admitting one.
RELEASE_SOURCE = r"(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)"
RELEASE_PATTERN = re.compile(RELEASE_SOURCE)
COMPARATOR_PATTERN = re.compile(r"(?P<operator>>=|<=|>|<|=)\s*" + RELEASE_SOURCE)

# The workspace manifest owns the SPDX expression and the author declaration
# the copyright holder is derived from. The crate manifests inherit that
# declaration and the npm manifests restate it; all of them must agree.
WORKSPACE_MANIFEST = Path("Cargo.toml")
PUBLISHED_PACKAGE_MANIFEST = Path("mcp-server/package.json")
INHERITING_CRATE_MANIFESTS = (
    Path("crates/jit/Cargo.toml"),
    Path("crates/server/Cargo.toml"),
)
AUTHOR_RESTATING_MANIFESTS = (
    PUBLISHED_PACKAGE_MANIFEST,
    Path("web/package.json"),
)
CARGO_INHERITED = {"workspace": True}
LICENSE_OPERATORS = frozenset({"OR", "AND", "WITH"})


class LicenseContract(NamedTuple):
    """Completeness contract for one SPDX license identifier.

    `phrases` are structural anchors spanning the whole text, so a truncated or
    paraphrased copy fails. `requires_declared_copyright` marks the licenses
    whose text carries the project's own copyright line, which has to name the
    holder the workspace manifest declares; the licenses that keep an upstream
    placeholder verbatim do not.
    """

    phrases: tuple[str, ...]
    requires_declared_copyright: bool


LICENSE_CONTRACTS: dict[str, LicenseContract] = {
    "MIT": LicenseContract(
        phrases=(
            "Permission is hereby granted, free of charge",
            "The above copyright notice and this permission notice shall be included",
            'THE SOFTWARE IS PROVIDED "AS IS"',
        ),
        requires_declared_copyright=True,
    ),
    "Apache-2.0": LicenseContract(
        phrases=(
            "Apache License",
            "Version 2.0, January 2004",
            "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",
            "END OF TERMS AND CONDITIONS",
            "APPENDIX: How to apply the Apache License to your work.",
        ),
        requires_declared_copyright=False,
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


def nested_value(value: dict[str, Any], *keys: str) -> Any:
    current: Any = value
    for key in keys:
        if not isinstance(current, dict) or key not in current:
            raise ValueError(f"missing {'.'.join(keys)}")
        current = current[key]
    return current


def nested_string(value: dict[str, Any], *keys: str) -> str:
    current = nested_value(value, *keys)
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


def read_copyright_holder(path: Path) -> str:
    """Return the copyright holder the workspace manifest's authors declare."""
    authors = nested_value(read_toml(path), "workspace", "package", "authors")
    if (
        not isinstance(authors, list)
        or not authors
        or not all(isinstance(author, str) and author for author in authors)
    ):
        raise ValueError(
            "workspace.package.authors is not a non-empty list of non-empty names"
        )
    return ", ".join(authors)


def copyright_line_pattern(holder: str) -> re.Pattern[str]:
    """Return the pattern a license's copyright line must match for `holder`."""
    return re.compile(rf"^Copyright \(c\) \d{{4}} {re.escape(holder)}$", re.MULTILINE)


def verify_author_declaration(root: Path, findings: list[str]) -> str | None:
    """Check every restatement of the declared author; return the holder it names."""
    try:
        holder = read_copyright_holder(root / WORKSPACE_MANIFEST)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        findings.append(f"copyright holder at {WORKSPACE_MANIFEST}: {error}")
        return None

    for relative in INHERITING_CRATE_MANIFESTS:
        try:
            inherited = nested_value(read_toml(root / relative), "package", "authors")
        except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
            findings.append(f"author inheritance at {relative}: {error}")
            continue
        if inherited != CARGO_INHERITED:
            findings.append(
                f"author inheritance at {relative} declares {inherited!r}; expected "
                f"the workspace inheritance {CARGO_INHERITED!r}"
            )

    for relative in AUTHOR_RESTATING_MANIFESTS:
        try:
            restated = nested_string(read_json(root / relative), "author")
        except (OSError, ValueError, json.JSONDecodeError) as error:
            findings.append(f"author at {relative}: {error}")
            continue
        if restated != holder:
            findings.append(
                f"author {restated!r} at {relative} differs from the declared "
                f"copyright holder {holder!r} at {WORKSPACE_MANIFEST}"
            )

    return holder


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


def verify_license_texts(
    root: Path, holder: str | None, findings: list[str]
) -> tuple[Path, ...]:
    """Check the license texts the manifest expression names; return their paths."""
    try:
        expression = nested_string(
            read_toml(root / WORKSPACE_MANIFEST),
            "workspace",
            "package",
            "license",
        )
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        findings.append(
            f"license expression at {WORKSPACE_MANIFEST}: {error}"
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
                f"differs from {expression!r} at {WORKSPACE_MANIFEST}"
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
        if (
            contract.requires_declared_copyright
            and holder is not None
            and not copyright_line_pattern(holder).search(text)
        ):
            findings.append(
                f"license text at {relative} has no copyright line naming the "
                f"declared holder (expected 'Copyright (c) <year> {holder}')"
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
            f"release note at {relative} carries a fenced block; it cites the "
            "canonical installation guide instead of repeating installation "
            "commands"
        )

    targets = linked_paths(relative, text)
    findings.extend(
        f"release note at {relative} does not link shipped license asset {asset}"
        for asset in license_paths
        if str(asset) not in targets
    )
    if str(COMPATIBILITY_PATH) not in targets:
        findings.append(f"release note at {relative} does not cite {COMPATIBILITY_PATH}")


class Comparator(NamedTuple):
    """One bound of a declared compatibility range."""

    operator: str
    bound: tuple[int, int, int]

    def admits(self, release: tuple[int, int, int]) -> bool:
        """Report whether one release satisfies this bound."""
        return {
            ">=": release >= self.bound,
            ">": release > self.bound,
            "<=": release <= self.bound,
            "<": release < self.bound,
            "=": release == self.bound,
        }[self.operator]


def release_of(match: re.Match[str]) -> tuple[int, int, int]:
    """Return the release a match's `major`, `minor`, and `patch` groups name."""
    major, minor, patch = (
        int(match.group(part)) for part in ("major", "minor", "patch")
    )
    return (major, minor, patch)


def parse_release(value: str) -> tuple[int, int, int]:
    """Parse one release, raising `ValueError` for any other version form."""
    match = RELEASE_PATTERN.fullmatch(value)
    if match is None:
        raise ValueError(f"{value!r} is not a major.minor.patch release")
    return release_of(match)


def parse_compatibility_range(declared: str) -> tuple[Comparator, ...]:
    """Parse the comma-separated comparators of a declared compatibility range.

    Raises `ValueError` for any comparator outside the forms this check reads,
    so an uninterpreted range fails rather than passes.
    """

    def comparator(part: str) -> Comparator:
        match = COMPARATOR_PATTERN.fullmatch(part.strip())
        if match is None:
            raise ValueError(
                f"comparator {part.strip()!r} is outside the forms this check "
                "reads: >=, >, <=, <, or = followed by a major.minor.patch release"
            )
        return Comparator(match.group("operator"), release_of(match))

    return tuple(map(comparator, declared.split(",")))


def range_admits(declared: str, version: str) -> bool:
    """Report whether a declared compatibility range admits one version.

    Raises `ValueError` when either side is outside what this check reads.
    """
    release = parse_release(version)
    return all(
        comparator.admits(release)
        for comparator in parse_compatibility_range(declared)
    )


def verify_profile_compatibility(
    root: Path, expected: str, findings: list[str]
) -> None:
    """Check that the embedded profile package admits the derived version."""
    try:
        declared = nested_string(
            read_toml(root / PROFILE_PACKAGE_MANIFEST), "profile", "jit"
        )
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        findings.append(
            f"profile compatibility range at {PROFILE_PACKAGE_MANIFEST}: {error}"
        )
        return

    try:
        admits = range_admits(declared, expected)
    except ValueError as error:
        findings.append(
            f"profile compatibility range {declared!r} at "
            f"{PROFILE_PACKAGE_MANIFEST} was not matched against product version "
            f"{expected}: {error}"
        )
        return

    if not admits:
        findings.append(
            f"profile compatibility range {declared!r} at "
            f"{PROFILE_PACKAGE_MANIFEST} excludes product version {expected}"
        )


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

    holder = verify_author_declaration(root, findings)
    license_paths = verify_license_texts(root, holder, findings)

    if expected is not None:
        verify_changelog(root, expected, findings)
        verify_release_note(root, expected, license_paths, findings)
        verify_profile_compatibility(root, expected, findings)

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
    parser.add_argument(
        "--declared",
        action="store_true",
        help="print the manifest-derived product version alone, for a caller "
        "that names a release input after it",
    )
    arguments = parser.parse_args(argv)

    expected, findings = verify(arguments.root.resolve(), arguments.tag)
    if findings:
        for finding in findings:
            print(f"release-version-contract: {finding}", file=sys.stderr)
        return 1

    # The version alone, and only from a tree whose declarations agree: a
    # caller that names an artifact or a release note after this value never
    # receives one the contract has not just verified.
    if arguments.declared:
        print(expected)
        return 0

    print(
        "release-version-contract: "
        f"product version {expected}; manifests, locks, tag, compatibility and "
        "upgrade record, license texts, changelog entry, release-note source, and "
        "embedded profile compatibility range agree"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
