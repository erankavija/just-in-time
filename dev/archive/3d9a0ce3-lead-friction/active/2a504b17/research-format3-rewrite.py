#!/usr/bin/env python3
"""Rewrite ~/Projects/research's .jit store to format 3. Stages and commits nothing.

Usage: research-format3-rewrite.py <repository-path>

One-off for that repository. On top of format3-rewrite.py's index conversion, it
sets a bare `lead` actor to `agent:lead`, spells issue timestamps with a Z suffix,
stores every issue and event record in canonical key order, drops the fields the
binary rejects from config.toml, replaces gates.json with gates.toml, and moves the
namespace limits config.toml declared into rules.toml, writing derived state with
`jit validate --fix`. A second run changes nothing. It replaces nothing until every
record has converted.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

BARE_ACTOR = "lead"
ACTOR = "agent:lead"
RETIRED_EVENT_TAGS = {
    "document_archived",
    "lifecycle_timestamps_backfilled",
    "profile_applied",
}
CONFIG_LINES_DROPPED = {
    "reject_malformed_labels = true      # block labels that violate namespace:value format\n",
    "enforce_namespace_registry = true   # reject labels whose namespace isn't declared below\n",
    "required = true\n",
    'values = ["epic", "story", "task", "bug", "spike", "chore", "milestone"]\n',
    'values = ["critical", "high", "normal", "low"]\n',
    "pattern = '^v\\d+\\.\\d+(\\.\\d+)?(-[a-zA-Z0-9.-]+)?$'\n",
    'values = ["wont-fix", "duplicate", "obsolete", "invalid"]\n',
    "# Label semantics — new repos start strict so convention drift is caught early.\n",
    "# Set either flag to false if you want looser enforcement during onboarding.\n",
    "# Legacy flag; prefer `required = true` on individual namespaces (see [namespaces.type]).\n",
    "# require_type_label = false\n",
    "# label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'\n",
    "#   values      — optional allowed-value enum; free-form if omitted\n",
    "#   pattern     — optional regex over the value portion (checked by `jit validate`)\n",
    "#   required    — optional; when true every issue must carry a label from this namespace\n",
}
GATE_FIELDS_DROPPED = {"auto", "example_integration"}
# The default rows for the namespaces config.toml declares. A rules.toml without
# them switches every default rule off. namespace-registry blocks writes, as
# enforce_namespace_registry did; the shipped row does not.
DEFAULT_RULES = '''[[rules]]
name = "label-format"
origin = "default"
description = "Every label must match the canonical `namespace:value` format (namespace lowercase-kebab, value non-empty). Blocks the write and fails validation."
severity = "error"
enforce = true
assert = { json-schema = "schemas/default-label-format.json" }

[[rules]]
name = "namespace-registry"
origin = "default"
description = "Every label's namespace must be declared in the namespace registry. An unknown namespace blocks the write and fails validation."
severity = "error"
enforce = true
assert = { json-schema = "schemas/default-namespace-registry.json" }

[[rules]]
name = "namespace-unique-priority"
origin = "default"
description = "At most one `priority:` label per issue: `priority` is a unique namespace. Blocks the write and fails validation."
severity = "error"
enforce = true
assert = { require-label = { label = "priority:*", min = 0, max = 1 } }

[[rules]]
name = "namespace-unique-resolution"
origin = "default"
description = "At most one `resolution:` label per issue: `resolution` is a unique namespace. Blocks the write and fails validation."
severity = "error"
enforce = true
assert = { require-label = { label = "resolution:*", min = 0, max = 1 } }

[[rules]]
name = "namespace-unique-team"
origin = "default"
description = "At most one `team:` label per issue: `team` is a unique namespace. Blocks the write and fails validation."
severity = "error"
enforce = true
assert = { require-label = { label = "team:*", min = 0, max = 1 } }

[[rules]]
name = "namespace-unique-type"
origin = "default"
description = "At most one `type:` label per issue: `type` is a unique namespace. Blocks the write and fails validation."
severity = "error"
enforce = true
assert = { require-label = { label = "type:*", min = 0, max = 1 } }

[[rules]]
name = "type-hierarchy-known"
origin = "default"
description = "Every `type:<value>` label must name a type declared in the configured type hierarchy. An unknown type fails validation but never blocks a write."
severity = "error"
enforce = false
assert = { json-schema = "schemas/default-type-hierarchy-known.json" }
'''
CUSTOM_RULES = r'''# Limits this repository declared in config.toml until format 3, which moved them here.

[[rules]]
name = "type-required"
severity = "error"
enforce = false
assert = { require-label = { label = "type:*", min = 1 } }

[[rules]]
name = "priority-allowed-values"
severity = "error"
enforce = false
assert = { label-value-pattern = { namespace = "priority", regex = '^(critical|high|normal|low)$' } }

[[rules]]
name = "resolution-allowed-values"
severity = "error"
enforce = false
assert = { label-value-pattern = { namespace = "resolution", regex = '^(wont-fix|duplicate|obsolete|invalid)$' } }

[[rules]]
name = "milestone-version-pattern"
severity = "error"
enforce = false
assert = { label-value-pattern = { namespace = "milestone", regex = '^v\d+\.\d+(\.\d+)?(-[a-zA-Z0-9.-]+)?$' } }
'''


def fail(message):
    sys.exit(f"research-format3-rewrite: {message}")


def replace_file(path, data):
    temp = path.with_name(f".{path.name}.format3-rewrite.tmp")
    with open(temp, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)


def compact(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def pretty(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, indent=2)


def rewritten_index(store):
    index = json.loads((store / "index.json").read_text())
    version = index.get("schema_version")
    if version == 3:
        return None
    if version != 2:
        fail(f"unexpected schema_version {version}")
    listed = {p.stem for p in (store / "issues").glob("*.json")}
    indexed = index.get("all_ids", [])
    if len(indexed) != len(set(indexed)) or set(indexed) != listed:
        fail(f"all_ids disagrees with the issue files: {set(indexed) ^ listed}")
    return pretty({"deleted_ids": index.get("deleted_ids", []), "schema_version": 3}).encode()


def utc_spelling(timestamp):
    """The same instant as the writer spells it, with a Z suffix."""
    if timestamp.endswith("Z"):
        return timestamp
    if not timestamp.endswith("+00:00"):
        fail(f"timestamp is not UTC: {timestamp}")
    return timestamp.removesuffix("+00:00") + "Z"


def repaired_issue(record):
    """The record with its timestamps respelled and each bare actor repaired, and how many it held."""
    statuses = record.get("gates_status") or {}
    bare = [gate for gate, status in statuses.items() if status.get("updated_by") == BARE_ACTOR]
    for gate in bare:
        statuses[gate]["updated_by"] = ACTOR
    for field in ("created_at", "updated_at"):
        record[field] = utc_spelling(record[field])
    return record, len(bare)


def converted_event(line):
    """The canonical line, or None when dropped, with its repaired actor count."""
    record = json.loads(line)
    if record.get("type") in RETIRED_EVENT_TAGS:
        return None, 0
    if record.get("type") == "issue_released" and "released_by" not in record:
        record["released_by"] = None
    bare = record.get("updated_by") == BARE_ACTOR
    if bare:
        record["updated_by"] = ACTOR
    return compact(record), int(bare)


def rewritten_events(store):
    original = (store / "events.jsonl").read_text()
    if not original.endswith("\n"):
        fail("the event log ends in a torn line")
    converted = [converted_event(line) for line in original.splitlines()]
    kept = [line for line, _ in converted if line is not None]
    ids = [json.loads(line)["id"] for line in kept]
    if len(ids) != len(set(ids)):
        fail(f"{len(ids) - len(set(ids))} duplicate event ids")
    data = ("\n".join(kept) + "\n").encode()
    return data, len(original.splitlines()) - len(kept), sum(n for _, n in converted)


def rewritten_config(store):
    lines = (store / "config.toml").read_text().splitlines(keepends=True)
    return "".join(line for line in lines if line not in CONFIG_LINES_DROPPED).encode()


def toml_string(value):
    return json.dumps(value, ensure_ascii=False)


def rendered_gate(gate):
    """One [[gates]] entry in the layout of the binary's serializer."""
    unknown = set(gate) - {
        "version", "key", "title", "description", "stage", "mode", "checker", "priority", "reserved",
    } - GATE_FIELDS_DROPPED
    if unknown or gate["reserved"]:
        fail(f"gate {gate['key']} holds {unknown or gate['reserved']}")
    lines = ["[[gates]]", f"version = {gate['version']}"]
    lines += [f"{field} = {toml_string(gate[field])}" for field in
              ("key", "title", "description", "stage", "mode")]
    lines.append(f"priority = {gate['priority']}")
    checker = gate.get("checker")
    if checker:
        if checker["type"] != "exec" or checker["working_dir"] is not None:
            fail(f"gate {gate['key']} has an unexpected checker")
        lines += ["", "[gates.checker]", 'type = "exec"',
                  f"command = {toml_string(checker['command'])}",
                  f"timeout_seconds = {checker['timeout_seconds']}",
                  f"pass_context = {'true' if checker['pass_context'] else 'false'}"]
        if "prompt_file" in checker:
            lines.append(f"prompt_file = {toml_string(checker['prompt_file'])}")
        lines += ["", "[gates.checker.env]"]
        lines += [f"{name} = {toml_string(value)}" for name, value in sorted(checker["env"].items())]
    lines += ["", "[gates.reserved]"]
    return "\n".join(lines) + "\n"


def rewritten_gates(store):
    gates = json.loads((store / "gates.json").read_text())["gates"]
    return "\n".join(rendered_gate(gates[key]) for key in sorted(gates)).encode()


def fix_derived_state(repository):
    subprocess.run(["jit", "validate", "--fix"], cwd=repository, check=True)


def write_rules(repository, store):
    """Default rows need their schemas first, which --fix writes only without those rows."""
    fix_derived_state(repository)
    replace_file(store / "rules.toml", CUSTOM_RULES.encode())
    fix_derived_state(repository)
    replace_file(store / "rules.toml", (DEFAULT_RULES + "\n" + CUSTOM_RULES).encode())
    fix_derived_state(repository)


def main():
    if len(sys.argv) != 2:
        fail("usage: research-format3-rewrite.py <repository-path>")
    store = Path(sys.argv[1]) / ".jit"
    if not (store / "index.json").is_file():
        fail(f"no index at {store / 'index.json'}")

    index = rewritten_index(store)
    issues = []
    issue_actors = 0
    for path in sorted((store / "issues").glob("*.json")):
        record, bare = repaired_issue(json.loads(path.read_text()))
        issue_actors += bare
        data = pretty(record).encode()
        if data != path.read_bytes():
            issues.append((path, data))
    events, dropped, event_actors = rewritten_events(store)
    config = rewritten_config(store)
    has_gates_json = (store / "gates.json").is_file()
    gates = rewritten_gates(store) if has_gates_json else None

    # Nothing is replaced until every record above decoded and converted.
    if index is not None:
        replace_file(store / "index.json", index)
    for path, data in issues:
        replace_file(path, data)
    if events != (store / "events.jsonl").read_bytes():
        replace_file(store / "events.jsonl", events)
    if config != (store / "config.toml").read_bytes():
        replace_file(store / "config.toml", config)
    if gates is not None:
        replace_file(store / "gates.toml", gates)
        (store / "gates.json").unlink()
    if not (store / "rules.toml").exists():
        write_rules(sys.argv[1], store)
    print(
        f"{store}: index {'rewritten' if index else 'already format 3'}; "
        f"{len(issues)} issue record(s) rewritten, {issue_actors} actor(s) repaired; "
        f"{dropped} event line(s) dropped, {event_actors} event actor(s) repaired; "
        f"gates {'converted' if gates else 'already gates.toml'}"
    )


if __name__ == "__main__":
    main()
