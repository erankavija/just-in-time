#!/usr/bin/env bash
set -euo pipefail

# docs-check-package-content — what this repository's packages may carry.
# Mechanical check M8 of the `docs-mechanical` gate.
#
# A package's content is read at the destination it is applied to, not here.
# Two consequences are checked over one assembly of each package:
#
#   1. a `@/kind/id` citation in package content must resolve from the
#      registries that same package installs. `docs-check-citations.sh` (M3)
#      deliberately leaves the `assets/` subtree alone for exactly that reason —
#      it resolves citations against THIS repository's live registries, which is
#      the wrong domain for content that answers elsewhere;
#   2. no packaged file may carry content the declarations in
#      `scripts/docs-package-content.toml` prohibit — a path out of this
#      checkout, a deferred example domain, a deictic phrase that resolves only
#      here, or a tool the workflow must not require.
#
# BOTH SIDES ARE DERIVED LIVE, and this file states neither:
#   * the packages are the directories under `profiles` that hold a manifest —
#     the checkout's package-source location (`PROFILE_PACKAGE_SOURCES`);
#   * each package's content is assembled by `jit profile capture`, so a live
#     asset is read exactly as the product draws it from the checkout;
#   * the registries a package installs are its `keyed-array` contributions,
#     and which registry answers which citation leader comes from the live
#     `item_kinds` config: a registry-first kind declares the registry table
#     its items live in, and its aliases are leaders for that same table.
#     A kind with no table (a markdown-first kind such as `charter`) installs
#     nothing, so a package citing one cites something its destination will not
#     have — reported, not skipped;
#   * the prohibited content is the manifest's, and which files it applies to is
#     whatever each package declares.
#
# A leader no item kind registers is not a project-scope citation (the reserved
# `issue` scope, and documented parse-error examples), so it is skipped — the
# same rule M3 applies.
#
# Usage:
#   docs-check-package-content.sh [MANIFEST]
#     (takes no footprint — targets are configured; the default manifest is
#      scripts/docs-package-content.toml)
#
# Exit codes:
#   0 — every packaged citation resolves, and no package carries prohibited
#       content
#   1 — one or more DANGLING / FORBIDDEN findings
#   2 — usage/environment error (missing tooling, no git work tree, a manifest
#       that cannot be read or does not conform, or a package that could not be
#       captured or read)

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
# The checkout's package-source location: the directory `jit profile capture`
# reads this repository's authored packages from.
PACKAGE_SOURCES = "profiles"
MANIFEST = "manifest.toml"
CITATION = re.compile(r"@/([a-z][a-z0-9-]*)/([A-Za-z0-9][A-Za-z0-9-]*)")
# A token is a maximal run of word characters, so a prohibited word is not
# reported inside a longer one.
TOKEN = re.compile(r"\w+")


def fail(message):
    print(f"{ME}: {message}", file=sys.stderr)
    sys.exit(2)


arguments = sys.argv[1:]
if len(arguments) > 1:
    print(f"{ME}: expected at most one manifest path", file=sys.stderr)
    print(f"usage: {ME}.sh [MANIFEST]", file=sys.stderr)
    sys.exit(2)
# Resolved before the chdir below, so a caller's relative manifest keeps
# meaning what it did where the caller stood.
manifest_path = os.path.abspath(arguments[0] if arguments else DEFAULT_MANIFEST)

for tool in ("git", "jit"):
    if shutil.which(tool) is None:
        fail(f"'{tool}' not found on PATH")


def run(*command):
    """Return a command's stdout, or fail with its stderr."""
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip() or "(no output)"
        fail(f"`{' '.join(command)}` failed: {detail}")
    return result.stdout


root = run("git", "rev-parse", "--show-toplevel").strip()
if not root:
    fail("not inside a git work tree (needed to reach the package sources)")
os.chdir(root)

# Citation leader -> the registry table its items are installed into. A kind
# that declares no registry table maps to None: nothing a package can install.
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

# The prohibited content, validated before any of it is used: a malformed
# declaration is a usage error, never a finding a reader might try to fix in a
# package.
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
# Capture destinations must be worktree paths, so the scratch tree lives under
# the ignored build directory and is removed however this check ends.
build = os.path.join(root, "target")
try:
    os.makedirs(build, exist_ok=True)
except OSError as error:
    fail(f"cannot create a scratch destination under {build}: {error}")
scratch = tempfile.mkdtemp(prefix=f"{ME}.", dir=build)
try:
    for package in packages:
        assembled = os.path.join(scratch, package)
        run(
            "jit",
            "profile",
            "capture",
            "--quiet",
            "--source",
            os.path.join(PACKAGE_SOURCES, package),
            "--destination",
            assembled,
        )
        shown = json.loads(
            run("jit", "profile", "show", "--quiet", "--profile", f"path:{assembled}", "--json")
        )
        manifest = shown["profiles"][0]["manifest"]

        # The registries this package installs: one keyed-array contribution
        # target per registry table, holding the ids it publishes there.
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
                fail(f"cannot read captured package source {package}/{source}: {error}")
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
