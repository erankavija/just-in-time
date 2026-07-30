#!/usr/bin/env bash
set -euo pipefail

# docs-check-canonical — canonical-home, command-snippet, and support-matrix check.
# Mechanical check M6 of the `docs-mechanical` gate.
#
# Every adopter-facing fact declared in the canonical-homes manifest has one
# home, one automation source, and one statement. This check fails when any of
# those three stops holding:
#
#   MISSING   the home file is gone, its declared anchor no longer resolves,
#             the home no longer states the fact, or no navigation page links
#             the home;
#   STALE     a declared automation or manifest source is gone, or no longer
#             carries the literal the home states — the page now promises
#             something the repository does not produce;
#   DUPLICATE a second scanned page states the same literal, so the fact has
#             two homes and one of them will rot.
#
# A binding may declare `[[fact.binding.exempt]]` entries, each naming one path
# and the reason the same literal there is a different statement rather than a
# copy of the canonical one — a contributor-facing command that happens to spell
# the same thing. An exemption is itself checked: if the exempt path stops
# stating the literal, the exemption has outlived its cause and is a MISSING
# finding, so the table cannot quietly accumulate dead entries.
#
# Both comparison sides are derived at run time: the pages, the automation, and
# the navigation are all read from the tree. The manifest declares only which
# fact is bound to which pair, never the fact's value.
#
# Usage:
#   docs-check-canonical.sh [MANIFEST]     check (default manifest below)
#   docs-check-canonical.sh --homes [MANIFEST]
#                                          print each declared home path, one
#                                          per line, for a caller that needs to
#                                          include the canonical pages in a
#                                          footprint of its own
# Paths in the manifest are repository-root relative and resolved against the
# working directory, so the check runs against whatever tree it is invoked in.
#
# Exit codes:
#   0 — every home resolves, is navigable, and states a live, unduplicated fact
#       (prints "OK: canonical homes resolve, and each fact is live and unique")
#   1 — one or more MISSING / STALE / DUPLICATE findings
#   2 — usage error, or a manifest that cannot be read or does not conform

exec python3 - "$@" <<'PY'
import os
import re
import sys
import tomllib

DEFAULT_MANIFEST = "scripts/docs-canonical-homes.toml"
HEADING = re.compile(r"#{1,6}\s+(.*)")
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")


def usage(message):
    print(f"docs-check-canonical: {message}", file=sys.stderr)
    print(
        "usage: docs-check-canonical.sh [--homes] [MANIFEST]",
        file=sys.stderr,
    )
    sys.exit(2)


def fail(message):
    print(f"docs-check-canonical: {message}", file=sys.stderr)
    sys.exit(2)


arguments = sys.argv[1:]
homes_only = "--homes" in arguments
positional = [argument for argument in arguments if argument != "--homes"]
if len(positional) > 1:
    usage("expected at most one manifest path")
manifest_path = positional[0] if positional else DEFAULT_MANIFEST

try:
    with open(manifest_path, "rb") as handle:
        manifest = tomllib.load(handle)
except (OSError, tomllib.TOMLDecodeError) as error:
    fail(f"cannot read manifest {manifest_path}: {error}")


def require_list(container, key, where):
    value = container.get(key, [])
    if not isinstance(value, list) or not all(
        isinstance(item, str) and item for item in value
    ):
        fail(f"{where}: {key} must be a list of non-empty strings")
    return value


facts = manifest.get("fact")
if not isinstance(facts, list) or not facts:
    fail(f"manifest {manifest_path} declares no [[fact]] entries")

scan_roots = require_list(manifest, "scan_roots", manifest_path)
navigation = require_list(manifest, "navigation", manifest_path)

# Validate the manifest before using it: a malformed declaration is a usage
# error, never a documentation finding that a reader might try to fix in prose.
declarations = []
for index, fact in enumerate(facts):
    where = f"{manifest_path} [[fact]] #{index + 1}"
    if not isinstance(fact, dict):
        fail(f"{where}: not a table")
    identifier = fact.get("id")
    home = fact.get("home")
    if not isinstance(identifier, str) or not identifier:
        fail(f"{where}: id must be a non-empty string")
    if not isinstance(home, str) or not home:
        fail(f"{where}: home must be a non-empty string")
    bindings = fact.get("binding")
    if not isinstance(bindings, list) or not bindings:
        fail(f"{where}: declares no [[fact.binding]] entries")
    parsed_bindings = []
    for position, binding in enumerate(bindings):
        binding_where = f"{where} binding #{position + 1}"
        if not isinstance(binding, dict):
            fail(f"{binding_where}: not a table")
        doc = binding.get("doc")
        if not isinstance(doc, str) or not doc:
            fail(f"{binding_where}: doc must be a non-empty string")
        sources = require_list(binding, "sources", binding_where)
        if not sources:
            fail(f"{binding_where}: sources must name at least one path")
        source_token = binding.get("source_token", doc)
        if not isinstance(source_token, str) or not source_token:
            fail(f"{binding_where}: source_token must be a non-empty string")
        exemptions = binding.get("exempt", [])
        if not isinstance(exemptions, list):
            fail(f"{binding_where}: exempt must be a list of tables")
        parsed_exemptions = {}
        for slot, exemption in enumerate(exemptions):
            exempt_where = f"{binding_where} exempt #{slot + 1}"
            if not isinstance(exemption, dict):
                fail(f"{exempt_where}: not a table")
            exempt_path = exemption.get("path")
            reason = exemption.get("reason")
            if not isinstance(exempt_path, str) or not exempt_path:
                fail(f"{exempt_where}: path must be a non-empty string")
            # An exemption without a stated reason is an unexplained hole in the
            # uniqueness rule, so the manifest cannot express one.
            if not isinstance(reason, str) or not reason:
                fail(f"{exempt_where}: reason must be a non-empty string")
            parsed_exemptions[os.path.normpath(exempt_path)] = reason
        parsed_bindings.append((doc, source_token, sources, parsed_exemptions))
    path, _, anchor = home.partition("#")
    declarations.append((identifier, path, anchor, parsed_bindings))

if homes_only:
    for path in dict.fromkeys(path for _, path, _, _ in declarations):
        print(path)
    sys.exit(0)

_texts = {}


def text_of(path):
    """Return a file's text, or None when it cannot be read."""
    if path not in _texts:
        try:
            _texts[path] = open(path, encoding="utf-8").read()
        except OSError:
            _texts[path] = None
    return _texts[path]


def github_slug(text):
    slug = re.sub(r"[^\w\s-]", "", text.strip().lower())
    return slug.replace(" ", "-").strip("-")


def heading_slugs(text):
    slugs, fence = set(), False
    for line in text.splitlines():
        if line.lstrip().startswith("```"):
            fence = not fence
            continue
        if fence:
            continue
        heading = HEADING.match(line)
        if heading:
            slugs.add(github_slug(heading.group(1)))
    return slugs


def walk_error(error):
    fail(f"cannot traverse scan root: {error}")


def markdown_under(root):
    if os.path.isfile(root):
        return [root]
    collected = []
    for directory, _, names in os.walk(root, onerror=walk_error):
        collected += [
            os.path.join(directory, name) for name in names if name.endswith(".md")
        ]
    return collected


for root in scan_roots:
    if not os.path.exists(root):
        fail(f"scan root does not exist: {root}")

home_paths = [path for _, path, _, _ in declarations]
scanned = dict.fromkeys(
    os.path.normpath(path)
    for root in scan_roots
    for path in markdown_under(root)
)
scanned.update(dict.fromkeys(os.path.normpath(path) for path in home_paths))

# Every intra-repository link target of every navigation page, root-relative,
# so a home is navigable if some navigation page links it.
navigable = set()
for page in navigation:
    text = text_of(page)
    if text is None:
        fail(f"navigation page does not exist: {page}")
    directory = os.path.dirname(page)
    navigable.update(
        os.path.normpath(os.path.join(directory, target.split("#", 1)[0]))
        for target in LINK.findall(text)
        if "://" not in target and not target.startswith("#")
    )

findings = []

for identifier, path, anchor, bindings in declarations:
    home_text = text_of(path)
    if home_text is None:
        findings.append(f"MISSING: {identifier}: canonical home {path} does not exist")
        continue
    if anchor and anchor not in heading_slugs(home_text):
        findings.append(
            f"MISSING: {identifier}: canonical home {path} has no heading anchor "
            f"#{anchor}"
        )
    if os.path.normpath(path) not in navigable:
        findings.append(
            f"MISSING: {identifier}: canonical home {path} is not linked from "
            f"navigation ({', '.join(navigation)})"
        )
    for doc, source_token, sources, exemptions in bindings:
        if doc not in home_text:
            findings.append(
                f"MISSING: {identifier}: canonical home {path} no longer states "
                f"{doc!r}"
            )
        for source in sources:
            source_text = text_of(source)
            if source_text is None:
                findings.append(
                    f"STALE: {identifier}: source {source} does not exist, so "
                    f"{path} states {doc!r} on nothing"
                )
            elif source_token not in source_text:
                findings.append(
                    f"STALE: {identifier}: source {source} no longer carries "
                    f"{source_token!r}, which {path} states as {doc!r}"
                )
        home_normalized = os.path.normpath(path)
        findings += [
            f"DUPLICATE: {identifier}: {other} also states {doc!r}; the canonical "
            f"home is {path}"
            for other in scanned
            if other != home_normalized
            and other not in exemptions
            and doc in (text_of(other) or "")
        ]
        # A declared exemption whose page stopped stating the literal has
        # outlived its cause and belongs out of the table.
        findings += [
            f"MISSING: {identifier}: exemption for {exempt_path} no longer applies "
            f"— it does not state {doc!r} ({reason})"
            for exempt_path, reason in exemptions.items()
            if doc not in (text_of(exempt_path) or "")
        ]

if findings:
    print("\n".join(findings))
    sys.exit(1)
print("OK: canonical homes resolve, and each fact is live and unique")
PY
