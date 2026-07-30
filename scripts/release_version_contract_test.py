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
UPGRADE_SECTION = "## Upgrade expectations"
LICENSE_EXPRESSION = "MIT OR Apache-2.0"
MIT_PHRASES = (
    "Permission is hereby granted, free of charge",
    "The above copyright notice and this permission notice shall be included",
    'THE SOFTWARE IS PROVIDED "AS IS"',
)
APACHE_PHRASES = (
    "Apache License",
    "Version 2.0, January 2004",
    "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",
    "END OF TERMS AND CONDITIONS",
    "APPENDIX: How to apply the Apache License to your work.",
)
AUTHOR = "Example Holder"
MIT_COPYRIGHT_LINE = f"Copyright (c) 2026 {AUTHOR}"
COMPATIBILITY_RECORD = "docs/reference/compatibility.md"


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

    def _edit(self, relative: str, old: str, new: str) -> None:
        path = self.root / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text, relative)
        path.write_text(text.replace(old, new), encoding="utf-8")

    @property
    def _release_note(self) -> str:
        return f"docs/release-notes/v{self.version}.md"

    def _write_fixture(self) -> None:
        self._write(
            "Cargo.toml",
            '[workspace]\nmembers = ["crates/jit", "crates/server"]\n\n'
            f'[workspace.package]\nlicense = "{LICENSE_EXPRESSION}"\n'
            f'authors = ["{AUTHOR}"]\n',
        )
        for directory, crate in (("jit", "jit"), ("server", "jit-server")):
            self._write(
                f"crates/{directory}/Cargo.toml",
                f'[package]\nname = "{crate}"\nversion = "{self.version}"\n'
                "authors.workspace = true\n",
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
            manifest: dict[str, object] = {
                "name": name,
                "version": self.version,
                "author": AUTHOR,
            }
            if directory == "mcp-server":
                manifest["license"] = LICENSE_EXPRESSION
            self._write_json(f"{directory}/package.json", manifest)
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
            COMPATIBILITY_RECORD,
            "# Product Compatibility\n\n"
            f"**Product compatibility version:** `{self.version}`\n\n"
            "Supported capabilities:\n\n"
            f"{capability_list}\n\n"
            f"{UPGRADE_SECTION}\n\n"
            "Adopters replace the installed artifacts wholesale.\n",
        )

        self._write(
            "LICENSE-MIT",
            f"MIT License\n\n{MIT_COPYRIGHT_LINE}\n\n"
            + "\n\n".join(MIT_PHRASES)
            + "\n",
        )
        self._write("LICENSE-APACHE", "\n\n".join(APACHE_PHRASES) + "\n")

        self._write(
            "CHANGELOG.md",
            "# Changelog\n\n## [Unreleased]\n\n"
            f"## [{self.version}] - 2026-07-30\n\n"
            "### Added\n\n- The first release.\n",
        )

        self._write(
            self._release_note,
            f"# JIT v{self.version}\n\n"
            "## Release assets\n\n"
            "- the native archive, carrying [LICENSE-MIT](../../LICENSE-MIT) and\n"
            "  [LICENSE-APACHE](../../LICENSE-APACHE)\n\n"
            "## Compatibility\n\n"
            "See [the compatibility record](../reference/compatibility.md).\n",
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

    def test_contract_rejects_duplicate_compatibility_declarations(self) -> None:
        declaration = self.root / "docs/reference/compatibility.md"
        declaration.write_text(
            declaration.read_text(encoding="utf-8")
            + f"\n**Product compatibility version:** `{self.version}`\n",
            encoding="utf-8",
        )

        self._assert_fails_with("expected exactly one")

    def test_contract_rejects_conflicting_compatibility_declarations(self) -> None:
        declaration = self.root / "docs/reference/compatibility.md"
        declaration.write_text(
            declaration.read_text(encoding="utf-8")
            + "\n**Product compatibility version:** `7.8.8`\n",
            encoding="utf-8",
        )

        self._assert_fails_with("conflicting product compatibility versions")

    def test_contract_rejects_missing_supported_capability(self) -> None:
        declaration = self.root / "docs/reference/compatibility.md"
        declaration.write_text(
            declaration.read_text(encoding="utf-8").replace(
                f"- {CAPABILITIES[-1]}\n", ""
            ),
            encoding="utf-8",
        )

        self._assert_fails_with("supported capability")

    def test_contract_rejects_missing_upgrade_expectations_record(self) -> None:
        self._edit(COMPATIBILITY_RECORD, UPGRADE_SECTION, "## Something else")

        self._assert_fails_with(UPGRADE_SECTION)

    def test_contract_rejects_missing_license_text(self) -> None:
        (self.root / "LICENSE-APACHE").unlink()

        self._assert_fails_with("LICENSE-APACHE")

    def test_contract_rejects_incomplete_license_text(self) -> None:
        self._edit("LICENSE-APACHE", APACHE_PHRASES[-1], "")

        self._assert_fails_with(APACHE_PHRASES[-1])

    def test_contract_rejects_unfilled_license_copyright_line(self) -> None:
        self._edit("LICENSE-MIT", MIT_COPYRIGHT_LINE, "Copyright (c) <year> <holders>")

        self._assert_fails_with("copyright line")

    def test_contract_rejects_license_copyright_line_naming_another_holder(
        self,
    ) -> None:
        self._edit("LICENSE-MIT", AUTHOR, "Someone Else")

        self._assert_fails_with(AUTHOR)

    def test_contract_rejects_missing_author_declaration(self) -> None:
        self._edit("Cargo.toml", f'authors = ["{AUTHOR}"]\n', "")

        self._assert_fails_with("copyright holder")

    def test_contract_rejects_empty_author_declaration(self) -> None:
        self._edit("Cargo.toml", f'authors = ["{AUTHOR}"]', "authors = []")

        self._assert_fails_with("copyright holder")

    def test_contract_rejects_crate_not_inheriting_the_author_declaration(self) -> None:
        self._edit("crates/server/Cargo.toml", "authors.workspace = true\n", "")

        self._assert_fails_with("crates/server/Cargo.toml")

    def test_contract_rejects_mismatched_node_author(self) -> None:
        self._edit("web/package.json", AUTHOR, "Someone Else")

        self._assert_fails_with("web/package.json")

    def test_contract_rejects_license_expression_without_a_completeness_contract(
        self,
    ) -> None:
        for relative, old, new in (
            ("Cargo.toml", LICENSE_EXPRESSION, "MIT OR Zlib"),
            ("mcp-server/package.json", LICENSE_EXPRESSION, "MIT OR Zlib"),
        ):
            self._edit(relative, old, new)

        self._assert_fails_with("Zlib")

    def test_contract_rejects_mismatched_license_expression(self) -> None:
        self._edit("mcp-server/package.json", LICENSE_EXPRESSION, "MIT")

        self._assert_fails_with("license expression")

    def test_contract_rejects_missing_changelog_release_entry(self) -> None:
        self._edit("CHANGELOG.md", f"## [{self.version}] - 2026-07-30", "## Notes")

        self._assert_fails_with("changelog")

    def test_contract_rejects_changelog_release_entry_for_another_version(self) -> None:
        self._edit("CHANGELOG.md", f"## [{self.version}]", "## [7.8.8]")

        self._assert_fails_with("7.8.8")

    def test_contract_rejects_missing_release_note_source(self) -> None:
        (self.root / self._release_note).unlink()

        self._assert_fails_with(self._release_note)

    def test_contract_rejects_release_note_source_naming_another_version(self) -> None:
        self._edit(self._release_note, f"# JIT v{self.version}", "# JIT v7.8.8")

        self._assert_fails_with(f"JIT v{self.version}")

    def test_contract_rejects_release_note_source_omitting_a_license_asset(
        self,
    ) -> None:
        self._edit(self._release_note, "(../../LICENSE-APACHE)", "(elsewhere)")

        self._assert_fails_with("LICENSE-APACHE")

    def test_contract_rejects_release_note_source_omitting_the_compatibility_record(
        self,
    ) -> None:
        self._edit(self._release_note, "(../reference/compatibility.md)", "(elsewhere)")

        self._assert_fails_with(COMPATIBILITY_RECORD)

    def test_contract_rejects_release_note_source_repeating_installation_commands(
        self,
    ) -> None:
        note = self.root / self._release_note
        note.write_text(
            note.read_text(encoding="utf-8") + "\n```sh\ncargo install jit\n```\n",
            encoding="utf-8",
        )

        self._assert_fails_with("installation commands")


if __name__ == "__main__":
    unittest.main()
