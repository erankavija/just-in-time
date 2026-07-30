#!/usr/bin/env python3
"""Regression tests for the product release-version contract."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


CHECKER = Path(__file__).with_name("release-version-contract.py")
CAPABILITIES = (
    "native `jit` CLI",
    "`jit-server` Rust server and API",
    "built web UI",
    "`@erankavija/jit-mcp-server` MCP server",
)


class ReleaseVersionContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scratch = tempfile.TemporaryDirectory(
            dir=os.environ.get("RELEASE_VERSION_TEST_TMPDIR")
        )
        self.root = Path(self.scratch.name)
        self.version = "7.8.9"
        self._write_fixture()

    def tearDown(self) -> None:
        self.scratch.cleanup()

    def _write(self, relative: str, content: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def _write_json(self, relative: str, value: object) -> None:
        self._write(relative, json.dumps(value, indent=2) + "\n")

    def _write_fixture(self) -> None:
        self._write(
            "crates/jit/Cargo.toml",
            f'[package]\nname = "jit"\nversion = "{self.version}"\n',
        )
        self._write(
            "crates/server/Cargo.toml",
            f'[package]\nname = "jit-server"\nversion = "{self.version}"\n',
        )
        self._write(
            "Cargo.lock",
            "version = 4\n\n"
            f'[[package]]\nname = "jit"\nversion = "{self.version}"\n\n'
            f'[[package]]\nname = "jit-server"\nversion = "{self.version}"\n',
        )

        for directory, name in (
            ("mcp-server", "@erankavija/jit-mcp-server"),
            ("web", "web"),
        ):
            self._write_json(
                f"{directory}/package.json",
                {"name": name, "version": self.version},
            )
            self._write_json(
                f"{directory}/package-lock.json",
                {
                    "name": name,
                    "version": self.version,
                    "lockfileVersion": 3,
                    "packages": {
                        "": {"name": name, "version": self.version},
                    },
                },
            )

        capability_list = "\n".join(f"- {capability}" for capability in CAPABILITIES)
        self._write(
            "docs/reference/compatibility.md",
            "# Product Compatibility\n\n"
            f"**Product compatibility version:** `{self.version}`\n\n"
            "Supported capabilities:\n\n"
            f"{capability_list}\n",
        )

        # Product and repository-format versions are separate contracts. The
        # release checker must neither compare nor rewrite this marker.
        self._write_json(
            ".jit/index.json",
            {"schema_version": 314, "all_ids": [], "deleted_ids": []},
        )

    def _run(self, tag: str | None = None) -> subprocess.CompletedProcess[str]:
        command = [sys.executable, str(CHECKER), "--root", str(self.root)]
        if tag is not None:
            command.extend(("--tag", tag))
        return subprocess.run(command, capture_output=True, check=False, text=True)

    def _assert_fails_with(self, expected: str, tag: str | None = None) -> None:
        result = self._run(tag)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(expected, result.stderr)

    def test_contract_derives_product_version_and_ignores_repository_format(self) -> None:
        result = self._run(f"v{self.version}")

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"product version {self.version}", result.stdout)
        self.assertNotIn("314", result.stdout + result.stderr)

    def test_contract_rejects_mismatched_rust_crate(self) -> None:
        manifest = self.root / "crates/server/Cargo.toml"
        manifest.write_text(
            manifest.read_text(encoding="utf-8").replace(self.version, "7.8.8"),
            encoding="utf-8",
        )

        self._assert_fails_with("jit-server manifest")

    def test_contract_rejects_mismatched_node_package(self) -> None:
        package = self.root / "web/package.json"
        value = json.loads(package.read_text(encoding="utf-8"))
        value["version"] = "7.8.8"
        package.write_text(json.dumps(value), encoding="utf-8")

        self._assert_fails_with("web manifest")

    def test_contract_rejects_mismatched_rust_lock(self) -> None:
        lock = self.root / "Cargo.lock"
        lock.write_text(
            lock.read_text(encoding="utf-8").replace(
                f'name = "jit-server"\nversion = "{self.version}"',
                'name = "jit-server"\nversion = "7.8.8"',
            ),
            encoding="utf-8",
        )

        self._assert_fails_with("jit-server lock metadata")

    def test_contract_rejects_mismatched_node_lock(self) -> None:
        lock = self.root / "mcp-server/package-lock.json"
        value = json.loads(lock.read_text(encoding="utf-8"))
        value["packages"][""]["version"] = "7.8.8"
        lock.write_text(json.dumps(value), encoding="utf-8")

        self._assert_fails_with("MCP server lock metadata")

    def test_contract_rejects_mismatched_tag(self) -> None:
        self._assert_fails_with("release tag", "v7.8.8")

    def test_contract_rejects_mismatched_compatibility_declaration(self) -> None:
        declaration = self.root / "docs/reference/compatibility.md"
        declaration.write_text(
            declaration.read_text(encoding="utf-8").replace(self.version, "7.8.8"),
            encoding="utf-8",
        )

        self._assert_fails_with("compatibility declaration")

    def test_contract_rejects_missing_supported_capability(self) -> None:
        declaration = self.root / "docs/reference/compatibility.md"
        declaration.write_text(
            declaration.read_text(encoding="utf-8").replace(
                f"- {CAPABILITIES[-1]}\n", ""
            ),
            encoding="utf-8",
        )

        self._assert_fails_with("supported capability")


if __name__ == "__main__":
    unittest.main()
