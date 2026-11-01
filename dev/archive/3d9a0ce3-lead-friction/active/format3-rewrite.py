#!/usr/bin/env python3
"""Rewrite one repository's .jit store to format 3. Commits nothing.

Usage: format3-rewrite.py <repository-path>

A format 2 index becomes {deleted_ids, schema_version: 3}; the script refuses
when its all_ids disagrees with the issue files. A format 3 index stays as it
is, so a re-run after merging a format 2 branch only converts event lines. Every
run drops event lines carrying a retired tag and writes "released_by": null on
issue_released lines lacking that key; null means recorded before the field
existed. It stages and commits nothing.
"""
import json
import os
import sys
from pathlib import Path

RETIRED_EVENT_TAGS = {
    "document_archived",
    "lifecycle_timestamps_backfilled",
    "profile_applied",
}


def fail(message):
    sys.exit(f"format3-rewrite: {message}")


def replace_file(path, data):
    temp = path.with_name(f".{path.name}.format3-rewrite.tmp")
    with open(temp, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)


def rewritten_index(store):
    index = json.loads((store / "index.json").read_text())
    version = index.get("schema_version")
    if version == 3:
        if "all_ids" in index:
            fail("format 3 index carries all_ids")
        return None
    if version != 2:
        fail(f"unexpected schema_version {version}")
    issues = store / "issues"
    listed = {p.stem for p in issues.glob("*.json")} if issues.is_dir() else set()
    indexed = index.get("all_ids", [])
    if len(indexed) != len(set(indexed)) or set(indexed) != listed:
        fail(
            "all_ids disagrees with the issue files: "
            f"only indexed {sorted(set(indexed) - listed)}, "
            f"only files {sorted(listed - set(indexed))}, "
            f"duplicates {len(indexed) - len(set(indexed))}"
        )
    # The key order and layout the binary's canonical writer produces.
    return json.dumps(
        {"deleted_ids": index.get("deleted_ids", []), "schema_version": 3}, indent=2
    ).encode()


def converted_line(line):
    """The line as format 3 holds it, or None when it is dropped."""
    try:
        record = json.loads(line)
    except ValueError:
        return line  # A torn line is the log's certified exception.
    if not isinstance(record, dict):
        return line
    kind = record.get("type")
    if kind in RETIRED_EVENT_TAGS:
        return None
    if kind != "issue_released" or "released_by" in record:
        return line
    ending = line[len(line.rstrip(b"\r\n")) :]
    canonical = json.dumps(record, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    if canonical.encode() + ending != line:
        fail(f"issue_released line is not canonical: {line!r}")
    record["released_by"] = None
    return (
        json.dumps(record, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()
        + ending
    )


def main():
    if len(sys.argv) != 2:
        fail("usage: format3-rewrite.py <repository-path>")
    store = Path(sys.argv[1]) / ".jit"
    if not (store / "index.json").is_file():
        fail(f"no index at {store / 'index.json'}")

    index = rewritten_index(store)
    events = store / "events.jsonl"
    original = events.read_bytes() if events.is_file() else b""
    lines = original.splitlines(keepends=True)
    converted = [converted_line(line) for line in lines]
    kept = [line for line in converted if line is not None]
    dropped = len(lines) - len(kept)
    released = sum(1 for old, new in zip(lines, converted) if new not in (None, old))

    if index is not None:
        replace_file(store / "index.json", index)
    if dropped or released:
        replace_file(events, b"".join(kept))
    state = "rewritten" if index is not None else "already format 3"
    print(
        f"{store}: index {state}; dropped {dropped} retired event line(s); "
        f"wrote released_by on {released} line(s)"
    )


if __name__ == "__main__":
    main()
