#!/usr/bin/env python3
"""Check the declared minimum supported Rust version against current stable.

The workspace declares one `rust-version`; every crate inherits it. This
command derives that declaration from `cargo metadata` and current stable from
the Rust release channel manifest, both at run time, and reports whether the
declaration still sits inside the supported recency window: current stable may
be at most `MAX_MINOR_LAG` minor releases ahead of it, and may not be behind it.
Patch releases are release-train maintenance rather than a stable-release
interval, so they leave the window where it is.

The verdict is written to stdout as one JSON object naming both versions, the
distance between them, and the window that distance was judged against, so a
caller can record the exact comparison it acted on. Findings are written to
stderr.

`docs/reference/release-policy.md` is the policy this command enforces and
states the refresh procedure a finding calls for.

Exit codes:
  0 — the declaration is inside the window
  1 — the declaration is outside the window
  2 — a version could not be derived, so no comparison was made
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from typing import Callable, NamedTuple
import urllib.error
import urllib.request


# rustup resolves the stable channel through this manifest, so it is the same
# release the toolchain a contributor installs would report.
STABLE_CHANNEL_URL = "https://static.rust-lang.org/dist/channel-rust-stable.toml"
STABLE_CHANNEL_TIMEOUT_SECONDS = 30

# How many minor releases current stable may be ahead of the declaration.
MAX_MINOR_LAG = 1

WITHIN_WINDOW = "within-window"
BEHIND_WINDOW = "behind-window"
AHEAD_OF_STABLE = "ahead-of-stable"

# A leading `major.minor[.patch]`, ending at the build metadata a released
# toolchain carries ("1.97.1 (8bab26f4f 2026-07-14)"). A channel name such as
# `stable` has no such prefix and is rejected: this policy compares releases.
RUST_VERSION_PATTERN = re.compile(r"^(\d+)\.(\d+)(?:\.(\d+))?(?=$|[\s+-])")


class PolicyError(Exception):
    """A version could not be derived, so no verdict is available.

    Distinct from a finding: an unreachable channel manifest or an unreadable
    manifest tree says nothing about whether the declaration is current, and
    reporting either as a pass or as a stale declaration would be a lie.
    """


class RustVersion(NamedTuple):
    """A Rust release, compared on its `major.minor` train.

    `patch` is retained for reporting and left out of every window decision.
    """

    major: int
    minor: int
    patch: int | None

    def __str__(self) -> str:
        components = (
            self.train if self.patch is None else (*self.train, self.patch)
        )
        return ".".join(str(component) for component in components)

    @property
    def train(self) -> tuple[int, int]:
        return (self.major, self.minor)


class Recency(NamedTuple):
    """Where a declaration sits relative to current stable.

    `minor_lag` is the number of minor releases stable is ahead within one
    major series, and is `None` across a major boundary, where minor numbers
    are not commensurable.
    """

    status: str
    minor_lag: int | None
    findings: tuple[str, ...]


def parse_rust_version(text: str) -> RustVersion:
    """Return the release a version string names.

    Raises `PolicyError` when the string names no release, which is what a
    moving channel name such as `stable` does.
    """
    match = RUST_VERSION_PATTERN.match(text.strip())
    if match is None:
        raise PolicyError(f"{text!r} names no Rust release")
    major, minor, patch = match.groups()
    return RustVersion(int(major), int(minor), None if patch is None else int(patch))


def evaluate_recency(declared: RustVersion, stable: RustVersion) -> Recency:
    """Judge a declaration against current stable. Pure: no I/O, no clock."""
    if declared.train > stable.train:
        return Recency(
            AHEAD_OF_STABLE,
            None,
            (
                f"declared Rust {declared} is newer than current stable {stable}; "
                "the workspace declares a compiler no release ships",
            ),
        )

    if declared.major != stable.major:
        return Recency(
            BEHIND_WINDOW,
            None,
            (
                f"declared Rust {declared} is a whole major series behind current "
                f"stable {stable}",
            ),
        )

    lag = stable.minor - declared.minor
    if lag > MAX_MINOR_LAG:
        return Recency(
            BEHIND_WINDOW,
            lag,
            (
                f"declared Rust {declared} is {lag} minor releases behind current "
                f"stable {stable}; the window allows {MAX_MINOR_LAG}",
            ),
        )

    return Recency(WITHIN_WINDOW, lag, ())


def read_channel_stable_version(payload: bytes) -> RustVersion:
    """Return the release the Rust channel manifest publishes as `rust`."""
    try:
        channel = tomllib.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise PolicyError(f"the release channel manifest is unreadable: {error}") from error

    version = channel.get("pkg", {}).get("rust", {}).get("version")
    if not isinstance(version, str) or not version:
        raise PolicyError("the release channel manifest names no `rust` package version")
    return parse_rust_version(version)


def current_stable_version(url: str = STABLE_CHANNEL_URL) -> RustVersion:
    """Fetch the release channel manifest and return the current stable release."""
    try:
        with urllib.request.urlopen(url, timeout=STABLE_CHANNEL_TIMEOUT_SECONDS) as response:
            payload = response.read()
    except (urllib.error.URLError, OSError, ValueError) as error:
        raise PolicyError(f"current stable is unreachable at {url}: {error}") from error
    return read_channel_stable_version(payload)


def cargo_metadata(root: Path) -> dict:
    """Return the workspace metadata cargo reports for the tree at `root`."""
    command = (
        "cargo",
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
        str(root / "Cargo.toml"),
    )
    try:
        result = subprocess.run(command, capture_output=True, check=False, text=True)
    except OSError as error:
        raise PolicyError(f"cargo metadata could not be run: {error}") from error
    if result.returncode != 0:
        raise PolicyError(f"cargo metadata failed: {result.stderr.strip()}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise PolicyError(f"cargo metadata returned no readable output: {error}") from error


def declared_rust_version(root: Path) -> RustVersion:
    """Return the one `rust-version` every workspace package declares.

    Raises `PolicyError` when a package declares none, or when the packages
    disagree: the enforced policy is a single declaration the whole workspace
    inherits, and either shape means part of the workspace sits outside it.
    """
    packages = cargo_metadata(root).get("packages")
    if not isinstance(packages, list) or not packages:
        raise PolicyError(f"cargo metadata reports no packages under {root}")

    undeclared = sorted(
        str(package.get("name")) for package in packages if not package.get("rust_version")
    )
    if undeclared:
        raise PolicyError(
            f"{', '.join(undeclared)} declare no rust-version; every crate inherits "
            "the workspace declaration"
        )

    declared = sorted({str(package["rust_version"]) for package in packages})
    if len(declared) > 1:
        raise PolicyError(
            f"workspace packages declare disagreeing rust-versions {', '.join(declared)}; "
            "the workspace declares one"
        )
    return parse_rust_version(declared[0])


def main(
    argv: list[str] | None = None,
    fetch_stable: Callable[[], RustVersion] = current_stable_version,
) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.cwd(),
        help="workspace root (default: current directory)",
    )
    parser.add_argument(
        "--declared",
        action="store_true",
        help="print the declared version alone, for a caller that installs that compiler",
    )
    arguments = parser.parse_args(argv)

    try:
        declared = declared_rust_version(arguments.root.resolve())
    except PolicyError as error:
        print(f"rust-version-policy: {error}", file=sys.stderr)
        return 2

    if arguments.declared:
        print(declared)
        return 0

    try:
        stable = fetch_stable()
    except PolicyError as error:
        print(f"rust-version-policy: {error}", file=sys.stderr)
        return 2

    recency = evaluate_recency(declared, stable)
    print(
        json.dumps(
            {
                "declared": str(declared),
                "stable": str(stable),
                "minor_lag": recency.minor_lag,
                "max_minor_lag": MAX_MINOR_LAG,
                "status": recency.status,
            }
        )
    )
    for finding in recency.findings:
        print(f"rust-version-policy: {finding}", file=sys.stderr)
    return 1 if recency.findings else 0


if __name__ == "__main__":
    raise SystemExit(main())
