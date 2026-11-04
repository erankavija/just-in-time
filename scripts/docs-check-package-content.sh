#!/usr/bin/env bash
set -euo pipefail

# docs-check-package-content — mechanical check M8 of `docs-mechanical`.
#
# Package content is read at the destination it is applied to, so a `@/kind/id`
# citation in it must resolve from the registries that same package installs —
# the domain `docs-check-citations.sh` (M3) cannot judge, which is why M3 skips
# `assets/`. Over the same assembly this also applies the manifest's content
# prohibitions. A citation leader no item kind registers is not a project-scope
# address and is skipped, as in M3.
#
# Usage:
#   docs-check-package-content.sh [MANIFEST]   (no footprint: targets are
#                                               configured; default manifest
#                                               scripts/docs-package-content.toml)
#
# Exit codes:
#   0 — citations resolve and no package carries prohibited content
#   1 — DANGLING / FORBIDDEN findings
#   2 — usage/environment error (tooling, work tree, manifest, or pack)

exec python3 - "$@" <<'PY'
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

ME = "docs-check-package-content"
DEFAULT_MANIFEST = "scripts/docs-package-content.toml"
PACKAGE_SOURCES = "profiles"  # the checkout's package-source location
MANIFEST = "manifest.toml"
CITATION = re.compile(r"@/([a-z][a-z0-9-]*)/([A-Za-z0-9][A-Za-z0-9-]*)")
# A maximal word run, so a prohibited word is not reported inside a longer one.
TOKEN = re.compile(r"\w+")


def fail(message):
    print(f"{ME}: {message}", file=sys.stderr)
    sys.exit(2)


arguments = sys.argv[1:]
if len(arguments) > 1:
    print(f"{ME}: expected at most one manifest path", file=sys.stderr)
    print(f"usage: {ME}.sh [MANIFEST]", file=sys.stderr)
    sys.exit(2)
# Absolute before the chdir below, so a relative manifest keeps its meaning.
manifest_path = os.path.abspath(arguments[0] if arguments else DEFAULT_MANIFEST)

for tool in ("git", "jit"):
    if shutil.which(tool) is None:
        fail(f"'{tool}' not found on PATH")


def run(*command, cwd=None):
    """Return a command's stdout, or fail with its stderr."""
    result = subprocess.run(command, capture_output=True, text=True, cwd=cwd)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip() or "(no output)"
        fail(f"`{' '.join(command)}` failed: {detail}")
    return result.stdout


root = run("git", "rev-parse", "--show-toplevel").strip()
if not root:
    fail("not inside a git work tree (needed to reach the package sources)")
os.chdir(root)

# Citation leader -> the registry table its items install into. A markdown-first
# kind declares no table, so nothing a package installs can answer it.
try:
    kinds = json.loads(run("jit", "config", "get", "item_kinds"))
except json.JSONDecodeError as error:
    fail(f"`jit config get item_kinds` did not answer JSON: {error}")

table_of_leader = {}
for kind, declaration in kinds.items():
    source = declaration.get("source")
    table = source.get("table") if isinstance(source, dict) else None
    for leader in [kind] + list(declaration.get("aliases") or []):
        table_of_leader[leader] = table

# Validated before use: a malformed declaration is a usage error, not a finding.
try:
    with open(manifest_path, "rb") as handle:
        declared = tomllib.load(handle)
except (OSError, tomllib.TOMLDecodeError) as error:
    fail(f"cannot read manifest {manifest_path}: {error}")

prohibitions = []
for index, entry in enumerate(declared.get("content") or []):
    where = f"{manifest_path} [[content]] #{index + 1}"
    if not isinstance(entry, dict):
        fail(f"{where}: not a table")
    topic = entry.get("topic")
    if not isinstance(topic, str) or not topic:
        fail(f"{where}: topic must be a non-empty string")
    rules = {}
    for key in ("forbid", "forbid_token"):
        value = entry.get(key, [])
        if not isinstance(value, list) or not all(
            isinstance(item, str) and item for item in value
        ):
            fail(f"{where}: {key} must be a list of non-empty strings")
        rules[key] = value
    if not (rules["forbid"] or rules["forbid_token"]):
        fail(f"{where}: declares no forbid or forbid_token entry")
    prohibitions.append((topic, rules["forbid"], rules["forbid_token"]))

if not prohibitions:
    fail(f"manifest {manifest_path} declares no [[content]] entries")

if not os.path.isdir(PACKAGE_SOURCES):
    fail(f"package sources are missing: {PACKAGE_SOURCES}")

packages = sorted(
    name
    for name in os.listdir(PACKAGE_SOURCES)
    if os.path.isfile(os.path.join(PACKAGE_SOURCES, name, MANIFEST))
)
if not packages:
    fail(f"{PACKAGE_SOURCES} publishes no profile package")

findings = []
# `profile add` publishes only inside a worktree, so packages are assembled in a
# throwaway repository under the system temp directory, never in this tree.
scratch = tempfile.mkdtemp(prefix=f"{ME}.")
try:
    assembly = os.path.join(scratch, "repo")
    os.mkdir(assembly)
    run("jit", "init", "--quiet", cwd=assembly)
    for package in packages:
        assembled = os.path.join(assembly, package)
        archive = os.path.join(scratch, f"{package}.tar")
        run(
            "jit",
            "profile",
            "pack",
            "--quiet",
            "--source",
            os.path.join(PACKAGE_SOURCES, package),
            "--output",
            archive,
        )
        run(
            "jit",
            "profile",
            "add",
            "--quiet",
            "--archive",
            archive,
            "--destination",
            package,
            cwd=assembly,
        )
        shown = json.loads(
            run(
                "jit", "profile", "show", "--quiet", "--profile", assembled, "--json",
                cwd=assembly,
            )
        )
        manifest = shown["profiles"][0]["manifest"]

        # What this package installs: ids per registry table.
        installed = {}
        for contribution in manifest.get("contribution") or []:
            if contribution.get("kind") == "keyed-array":
                installed.setdefault(contribution["target"], set()).add(
                    contribution["identity"]
                )

        content = [
            (entry["source"], entry["target"])
            for key in ("asset", "region")
            for entry in manifest.get(key) or []
        ]
        for source, target in content:
            try:
                with open(os.path.join(assembled, source), "rb") as handle:
                    text = handle.read().decode("utf-8", errors="ignore")
            except OSError as error:
                fail(f"cannot read packed package source {package}/{source}: {error}")
            for leader, identifier in dict.fromkeys(CITATION.findall(text)):
                if leader not in table_of_leader:
                    continue
                table = table_of_leader[leader]
                if identifier not in installed.get(table, set()):
                    findings.append(
                        f"DANGLING: {package}: {target} cites unresolved package "
                        f"item @/{leader}/{identifier}"
                    )

            tokens = None
            for topic, forbidden, forbidden_tokens in prohibitions:
                findings += [
                    f"FORBIDDEN: {package}: {target} carries {phrase!r} ({topic})"
                    for phrase in forbidden
                    if phrase in source or phrase in target or phrase in text
                ]
                if forbidden_tokens:
                    if tokens is None:
                        tokens = set(TOKEN.findall(text))
                    findings += [
                        f"FORBIDDEN: {package}: {target} names {word!r} ({topic})"
                        for word in forbidden_tokens
                        if word in tokens
                    ]
finally:
    shutil.rmtree(scratch, ignore_errors=True)

if findings:
    print("\n".join(dict.fromkeys(findings)))
    sys.exit(1)
print("OK: packaged citations resolve and no package carries prohibited content")
PY
