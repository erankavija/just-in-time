#!/usr/bin/env python3
"""Regression tests for the Rust version policy.

The comparison the policy turns on is pure: two parsed versions in, one verdict
out. It is exercised directly here, so the recency fixtures need neither a
network fetch nor an installed toolchain. Deriving the two versions —
`cargo metadata` for the declaration, the release channel manifest for current
stable — is covered separately, and the command's own wiring is driven through
`main` with the fetch step stubbed.
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest


def _load_policy():
    """Import the hyphenated command module under a Python-legal name.

    Bytecode writing is turned off first, so running the tests leaves no
    `__pycache__` beside the committed scripts.
    """
    sys.dont_write_bytecode = True
    path = Path(__file__).with_name("rust-version-policy.py")
    spec = importlib.util.spec_from_file_location("rust_version_policy", path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


policy = _load_policy()

FIXTURE_DECLARATION = "1.80"
CHANNEL_MANIFEST = """\
manifest-version = "2"
date = "2026-07-16"

[pkg.cargo]
version = "0.43.0 (c980f4866 2026-06-30)"

[pkg.rust]
version = "1.43.2 (8bab26f4f 2026-07-14)"
git_commit_hash = "8bab26f4f68e0e26f0bb7960be334d5b520ea452"
"""


def version(text: str) -> "policy.RustVersion":
    return policy.parse_rust_version(text)


class RecencyWindowTests(unittest.TestCase):
    """The pure comparison, over the four states a declaration can be in."""

    def test_evaluate_recency_accepts_a_declaration_matching_current_stable(self) -> None:
        recency = policy.evaluate_recency(version("1.97"), version("1.97.1"))

        self.assertEqual(recency.status, policy.WITHIN_WINDOW)
        self.assertEqual(recency.minor_lag, 0)
        self.assertEqual(recency.findings, ())

    def test_evaluate_recency_accepts_a_declaration_one_minor_release_behind_stable(
        self,
    ) -> None:
        recency = policy.evaluate_recency(version("1.96"), version("1.97.0"))

        self.assertEqual(recency.status, policy.WITHIN_WINDOW)
        self.assertEqual(recency.minor_lag, 1)
        self.assertEqual(recency.findings, ())

    def test_evaluate_recency_rejects_a_declaration_more_than_one_minor_release_behind_stable(
        self,
    ) -> None:
        recency = policy.evaluate_recency(version("1.95"), version("1.97.0"))

        self.assertEqual(recency.status, policy.BEHIND_WINDOW)
        self.assertEqual(recency.minor_lag, 2)
        self.assertTrue(recency.findings)
        self.assertTrue(
            all("1.95" in finding and "1.97.0" in finding for finding in recency.findings),
            recency.findings,
        )

    def test_evaluate_recency_rejects_a_declaration_newer_than_current_stable(self) -> None:
        recency = policy.evaluate_recency(version("1.98"), version("1.97.1"))

        self.assertEqual(recency.status, policy.AHEAD_OF_STABLE)
        self.assertTrue(recency.findings)
        self.assertTrue(
            all("1.98" in finding and "1.97.1" in finding for finding in recency.findings),
            recency.findings,
        )

    def test_evaluate_recency_spends_no_window_on_a_patch_release(self) -> None:
        """A patch release is not a stable-release interval, so it moves nothing."""
        early = policy.evaluate_recency(version("1.96"), version("1.97.0"))
        late = policy.evaluate_recency(version("1.96"), version("1.97.9"))

        self.assertEqual(late.status, early.status)
        self.assertEqual(late.minor_lag, early.minor_lag)

    def test_evaluate_recency_rejects_a_declaration_from_an_earlier_major_series(
        self,
    ) -> None:
        recency = policy.evaluate_recency(version("1.97"), version("2.0.0"))

        self.assertEqual(recency.status, policy.BEHIND_WINDOW)
        self.assertIsNone(recency.minor_lag)
        self.assertTrue(recency.findings)

    def test_evaluate_recency_rejects_a_declaration_from_a_later_major_series(self) -> None:
        recency = policy.evaluate_recency(version("2.0"), version("1.97.1"))

        self.assertEqual(recency.status, policy.AHEAD_OF_STABLE)
        self.assertIsNone(recency.minor_lag)
        self.assertTrue(recency.findings)


class VersionParsingTests(unittest.TestCase):
    def test_parse_rust_version_reads_a_release_carrying_build_metadata(self) -> None:
        parsed = policy.parse_rust_version("1.97.1 (8bab26f4f 2026-07-14)")

        self.assertEqual((parsed.major, parsed.minor, parsed.patch), (1, 97, 1))
        self.assertEqual(str(parsed), "1.97.1")

    def test_parse_rust_version_reads_a_declaration_without_a_patch_component(self) -> None:
        parsed = policy.parse_rust_version("1.97")

        self.assertIsNone(parsed.patch)
        self.assertEqual(str(parsed), "1.97")

    def test_parse_rust_version_rejects_a_moving_channel_name(self) -> None:
        with self.assertRaises(policy.PolicyError):
            policy.parse_rust_version("stable")

    def test_read_channel_stable_version_reads_the_release_the_manifest_names(self) -> None:
        stable = policy.read_channel_stable_version(CHANNEL_MANIFEST.encode("utf-8"))

        self.assertEqual(str(stable), "1.43.2")

    def test_read_channel_stable_version_rejects_a_manifest_without_a_rust_package(
        self,
    ) -> None:
        truncated = CHANNEL_MANIFEST.split("[pkg.rust]")[0]

        with self.assertRaises(policy.PolicyError):
            policy.read_channel_stable_version(truncated.encode("utf-8"))


class WorkspaceFixture(unittest.TestCase):
    """A miniature two-crate workspace `cargo metadata` can be pointed at."""

    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory(
            dir=os.environ.get("RUST_VERSION_POLICY_TEST_TMPDIR")
        )
        self.root = Path(self.scratch.name)
        self._write(
            "Cargo.toml",
            '[workspace]\nmembers = ["crates/alpha", "crates/beta"]\nresolver = "2"\n\n'
            f'[workspace.package]\nedition = "2021"\n'
            f'rust-version = "{FIXTURE_DECLARATION}"\n',
        )
        for member in ("alpha", "beta"):
            self._write(
                f"crates/{member}/Cargo.toml",
                f'[package]\nname = "{member}"\nversion = "0.1.0"\n'
                "edition.workspace = true\nrust-version.workspace = true\n",
            )
            self._write(f"crates/{member}/src/lib.rs", "")

    def tearDown(self) -> None:
        self.scratch.cleanup()

    def _write(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


class DeclaredVersionTests(WorkspaceFixture):
    def test_declared_rust_version_derives_the_inherited_declaration_from_cargo_metadata(
        self,
    ) -> None:
        declared = policy.declared_rust_version(self.root)

        self.assertEqual(str(declared), FIXTURE_DECLARATION)

    def test_declared_rust_version_rejects_a_package_that_declares_nothing(self) -> None:
        self._write(
            "crates/beta/Cargo.toml",
            '[package]\nname = "beta"\nversion = "0.1.0"\nedition.workspace = true\n',
        )

        with self.assertRaises(policy.PolicyError) as raised:
            policy.declared_rust_version(self.root)

        self.assertIn("beta", str(raised.exception))

    def test_declared_rust_version_rejects_packages_that_declare_different_versions(
        self,
    ) -> None:
        self._write(
            "crates/beta/Cargo.toml",
            '[package]\nname = "beta"\nversion = "0.1.0"\n'
            'edition.workspace = true\nrust-version = "1.79"\n',
        )

        with self.assertRaises(policy.PolicyError) as raised:
            policy.declared_rust_version(self.root)

        self.assertIn("1.79", str(raised.exception))


class CommandTests(WorkspaceFixture):
    """The command's own wiring: output, exit status, and the fetch boundary."""

    def _run(self, *arguments: str, stable: str | None = "1.81.0"):
        calls: list[int] = []

        def fetch_stable() -> "policy.RustVersion":
            calls.append(1)
            if stable is None:
                raise policy.PolicyError("current stable is unreachable")
            return policy.parse_rust_version(stable)

        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            status = policy.main(
                ["--root", str(self.root), *arguments], fetch_stable=fetch_stable
            )
        return status, out.getvalue(), err.getvalue(), len(calls)

    def test_main_emits_the_declared_and_derived_stable_versions_as_json(self) -> None:
        status, out, _, _ = self._run(stable="1.81.0")
        verdict = json.loads(out)

        self.assertEqual(status, 0)
        self.assertEqual(verdict["declared"], FIXTURE_DECLARATION)
        self.assertEqual(verdict["stable"], "1.81.0")
        self.assertEqual(verdict["status"], policy.WITHIN_WINDOW)
        self.assertEqual(verdict["minor_lag"], 1)

    def test_main_reports_the_stable_release_its_fetch_step_supplies(self) -> None:
        """The stable side of the verdict comes from the fetch, not from the source."""
        _, out, _, _ = self._run(stable="1.80.7")

        self.assertEqual(json.loads(out)["stable"], "1.80.7")

    def test_main_fails_and_still_emits_a_verdict_for_a_stale_declaration(self) -> None:
        status, out, err, _ = self._run(stable="1.83.0")
        verdict = json.loads(out)

        self.assertEqual(status, 1)
        self.assertEqual(verdict["status"], policy.BEHIND_WINDOW)
        self.assertIn(FIXTURE_DECLARATION, err)

    def test_main_fails_for_a_declaration_newer_than_current_stable(self) -> None:
        status, out, _, _ = self._run(stable="1.79.0")

        self.assertEqual(status, 1)
        self.assertEqual(json.loads(out)["status"], policy.AHEAD_OF_STABLE)

    def test_main_separates_an_undetermined_verdict_from_a_stale_declaration(self) -> None:
        status, out, err, _ = self._run(stable=None)

        self.assertEqual(status, 2)
        self.assertEqual(out, "")
        self.assertIn("current stable", err)

    def test_main_prints_the_declaration_alone_without_deriving_stable(self) -> None:
        status, out, _, fetches = self._run("--declared")

        self.assertEqual(status, 0)
        self.assertEqual(out.strip(), FIXTURE_DECLARATION)
        self.assertEqual(fetches, 0)

    def test_main_reports_an_underivable_declaration_as_an_environment_error(self) -> None:
        (self.root / "Cargo.toml").unlink()

        status, out, err, fetches = self._run()

        self.assertEqual(status, 2)
        self.assertEqual(out, "")
        self.assertTrue(err)
        self.assertEqual(fetches, 0)


if __name__ == "__main__":
    unittest.main()
