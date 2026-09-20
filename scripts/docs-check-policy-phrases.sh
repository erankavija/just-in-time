#!/usr/bin/env bash
set -euo pipefail

# docs-check-policy-phrases — declared policy phrases in this repository's prose.
# Mechanical check M9 of the `docs-mechanical` gate.
#
# The contributor instructions, the gate prompts they drive, and the pages that
# document who owns the review contract carry policy whose wording is the
# policy: a phrase that quietly disappears changes what a reviewer is told to
# do, and a superseded rule that quietly returns reinstates a burden that was
# removed on purpose. Neither shows up as a broken link, a stale projection or
# a dangling citation, so no other mechanical check sees it.
#
# The manifest declares which literal is load-bearing in which artifact, and
# whether it must be present, absent, or present exactly once. This file states
# no phrase and no path: it is the comparison, the manifest is the declaration,
# and the artifacts under check are read from the tree.
#
# Usage:
#   docs-check-policy-phrases.sh [MANIFEST]
#     (takes no footprint — targets are configured; the default manifest is
#      scripts/docs-policy-phrases.toml)
#
# Exit codes:
#   0 — every declared phrase holds
#       (prints "OK: declared policy phrases hold")
#   1 — one or more MISSING / FORBIDDEN / COUNT findings
#   2 — usage error, or a manifest that cannot be read or does not conform

exec python3 - "$@" <<'PY'
import os
import re
import sys
import tomllib

ME = "docs-check-policy-phrases"
DEFAULT_MANIFEST = "scripts/docs-policy-phrases.toml"
WHITESPACE = re.compile(r"\s+")
# `name[field=value]`: the array element of `name` whose `field` equals `value`.
SELECTOR = re.compile(r"^([A-Za-z0-9_-]+)(?:\[([A-Za-z0-9_-]+)=([^\]]+)\])?$")


def fail(message):
    print(f"{ME}: {message}", file=sys.stderr)
    sys.exit(2)


def usage(message):
    print(f"{ME}: {message}", file=sys.stderr)
    print(f"usage: {ME}.sh [MANIFEST]", file=sys.stderr)
    sys.exit(2)


arguments = sys.argv[1:]
if len(arguments) > 1:
    usage("expected at most one manifest path")
manifest_path = arguments[0] if arguments else DEFAULT_MANIFEST

try:
    with open(manifest_path, "rb") as handle:
        manifest = tomllib.load(handle)
except (OSError, tomllib.TOMLDecodeError) as error:
    fail(f"cannot read manifest {manifest_path}: {error}")


def require_phrases(entry, key, where):
    value = entry.get(key, [])
    if not isinstance(value, list) or not all(
        isinstance(phrase, str) and phrase for phrase in value
    ):
        fail(f"{where}: {key} must be a list of non-empty strings")
    return value


def require_text(entry, key, where):
    value = entry.get(key)
    if not isinstance(value, str) or not value:
        fail(f"{where}: {key} must be a non-empty string")
    return value


# Validate the whole manifest before using any of it: a malformed declaration is
# a usage error, never a finding a reader might try to fix in prose.
documents = []
for index, entry in enumerate(manifest.get("document") or []):
    where = f"{manifest_path} [[document]] #{index + 1}"
    if not isinstance(entry, dict):
        fail(f"{where}: not a table")
    path = require_text(entry, "path", where)
    topic = require_text(entry, "topic", where)
    normalize = entry.get("normalize", False)
    if not isinstance(normalize, bool):
        fail(f"{where}: normalize must be a boolean")
    section = entry.get("section")
    if section is not None:
        if not isinstance(section, dict) or set(section) - {"after", "before"}:
            fail(f"{where}: section must be a table of after and optional before")
        section = (
            require_text(section, "after", where),
            require_text(section, "before", where) if "before" in section else None,
        )
    required = require_phrases(entry, "require", where)
    forbidden = require_phrases(entry, "forbid", where)
    once = require_phrases(entry, "exactly_once", where)
    # An entry that declares nothing asserts nothing, and would sit in the
    # manifest looking like coverage.
    if not (required or forbidden or once):
        fail(f"{where}: declares no require, forbid, or exactly_once phrase")
    documents.append((path, topic, normalize, section, required, forbidden, once))

values = []
for index, entry in enumerate(manifest.get("value") or []):
    where = f"{manifest_path} [[value]] #{index + 1}"
    if not isinstance(entry, dict):
        fail(f"{where}: not a table")
    file_path = require_text(entry, "file", where)
    topic = require_text(entry, "topic", where)
    select = require_text(entry, "select", where)
    required = require_phrases(entry, "require", where)
    if not required:
        fail(f"{where}: require must name at least one phrase")
    steps = []
    for segment in select.split("."):
        parsed = SELECTOR.match(segment)
        if not parsed:
            fail(f"{where}: select segment {segment!r} is not name or name[field=value]")
        steps.append(parsed.groups())
    values.append((file_path, topic, select, steps, required))

if not documents and not values:
    fail(f"manifest {manifest_path} declares no [[document]] or [[value]] entries")

_texts = {}


def text_of(path):
    """Return a file's text, or None when it cannot be read."""
    if path not in _texts:
        try:
            with open(path, encoding="utf-8") as handle:
                _texts[path] = handle.read()
        except OSError:
            _texts[path] = None
    return _texts[path]


def collapse(text):
    return WHITESPACE.sub(" ", text)


findings = []

for path, topic, normalize, section, required, forbidden, once in documents:
    text = text_of(path)
    if text is None:
        findings.append(f"MISSING: {path} ({topic}): the artifact cannot be read")
        continue
    if section is not None:
        after, before = section
        _, marker, text = text.partition(after)
        if not marker:
            findings.append(f"MISSING: {path} ({topic}): has no section opening {after!r}")
            continue
        if before is not None:
            text, marker, _ = text.partition(before)
            if not marker:
                findings.append(
                    f"MISSING: {path} ({topic}): section {after!r} is not bounded by {before!r}"
                )
                continue
    haystack = collapse(text) if normalize else text
    needle = collapse if normalize else (lambda phrase: phrase)
    findings += [
        f"MISSING: {path} ({topic}): does not state {phrase!r}"
        for phrase in required
        if needle(phrase) not in haystack
    ]
    findings += [
        f"FORBIDDEN: {path} ({topic}): states {phrase!r}, which this policy removed"
        for phrase in forbidden
        if needle(phrase) in haystack
    ]
    findings += [
        f"COUNT: {path} ({topic}): states {phrase!r} {count} times, expected exactly once"
        for phrase, count in ((phrase, haystack.count(needle(phrase))) for phrase in once)
        if count != 1
    ]


def resolve(document, steps):
    """Walk `steps` through `document`, or return None where the walk ends."""
    cursor = document
    for name, field, wanted in steps:
        if not isinstance(cursor, dict) or name not in cursor:
            return None
        cursor = cursor[name]
        if field is None:
            continue
        if not isinstance(cursor, list):
            return None
        cursor = next(
            (
                element
                for element in cursor
                if isinstance(element, dict) and element.get(field) == wanted
            ),
            None,
        )
        if cursor is None:
            return None
    return cursor


for file_path, topic, select, steps, required in values:
    if not os.path.exists(file_path):
        findings.append(f"MISSING: {file_path} ({topic}): the file cannot be read")
        continue
    try:
        with open(file_path, "rb") as handle:
            document = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        findings.append(f"MISSING: {file_path} ({topic}): does not parse as TOML: {error}")
        continue
    resolved = resolve(document, steps)
    if not isinstance(resolved, str):
        findings.append(f"MISSING: {file_path} ({topic}): {select} resolves to no string")
        continue
    findings += [
        f"MISSING: {file_path} ({topic}): {select} does not carry {phrase!r}"
        for phrase in required
        if phrase not in resolved
    ]

if findings:
    print("\n".join(findings))
    sys.exit(1)
print("OK: declared policy phrases hold")
PY
