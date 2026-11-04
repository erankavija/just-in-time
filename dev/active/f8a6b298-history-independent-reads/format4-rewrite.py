#!/usr/bin/env python3
"""Rewrite one checkout's .jit data root and <common-dir>/jit to format 4.

Usage: format4-rewrite.py <checkout>

It plans every change before writing any and refuses, changing nothing, on a
condition it cannot convert. A second run changes no byte, and a run over a
tree merged from a format 3 branch converts only what the branch added. It
stages and commits nothing and takes no jit lock: run it only while no tracker
writer works in the repository.

Format 4 against format 3:
  - gate-runs/<run-id>/result.json moves to gate-runs/<issue-id>/<run-id>.json
    byte for byte, except that a reused verdict's origin gains source_issue;
  - a gate state with runs and no run_id names the run of that gate with the
    greatest (started_at, run_id);
  - gate-runs/verdicts/<gate-key>/<digest>.json names the latest qualifying run;
  - archive-evidence/<target-key>.json holds each target's executions in log order;
  - profile_lifecycle lines lose isolated_torn_tail;
  - <common-dir>/jit/claims.index.json becomes leases/<issue-id>.json records
    and claims.seq, and an empty heartbeat/ goes.
New records take the binary writer's form: serde_json pretty output with
sorted keys (lease records in field order), no trailing newline, and numbers
as stored.
"""
import calendar
import hashlib
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

INDEX_VERSION = 4
REUSABLE_STATUSES = {"passed", "failed"}
ORIGINS = {"executed", "reused", "attested"}
EXECUTION_FIELDS = (
    "target",
    "destination_root",
    "publications",
    "reference_changes",
    "planned_deletions",
    "reconciling",
)
LEASE_FIELDS = (
    "lease_id",
    "issue_id",
    "agent_id",
    "worktree_id",
    "branch",
    "ttl_secs",
    "acquired_at",
    "expires_at",
    "last_beat",
)
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
TIMESTAMP = re.compile(
    r"^(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)(?:\.(\d{1,9}))?(Z|[+-]\d\d:\d\d)$"
)
TORN_TAIL_MARKERS = (b'"isolated_torn_tail":false,', b',"isolated_torn_tail":false')
REUSED_ORIGIN = re.compile(
    r'\n( +)"origin": \{\n( +)"derivation": "reused",\n\2("source_run": "[^"\\]*")\n\1\}'
)


class Refusal(Exception):
    pass


class Number:
    """A JSON number kept as its stored text, which serde reproduces."""

    __slots__ = ("text",)

    def __init__(self, text):
        self.text = text

    def __eq__(self, other):
        return isinstance(other, Number) and other.text == self.text

    def __hash__(self):
        return hash(self.text)

    def __int__(self):
        return int(self.text)


def refuse_constant(name):
    raise Refusal(f"non-standard JSON constant {name}")


def parse(data, where):
    try:
        return json.loads(data, parse_int=Number, parse_float=Number, parse_constant=refuse_constant)
    except ValueError as error:
        raise Refusal(f"{where}: does not parse: {error}") from None


def dump(value, pretty=True, sort=True, depth=0):
    """serde_json's to_vec_pretty (or to_vec) of a parsed value."""
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, Number):
        return value.text
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    separator = ": " if pretty else ":"
    if isinstance(value, list):
        opening, closing = "[", "]"
        items = [dump(item, pretty, sort, depth + 1) for item in value]
    elif isinstance(value, dict):
        opening, closing = "{", "}"
        items = [
            json.dumps(key, ensure_ascii=False) + separator + dump(value[key], pretty, sort, depth + 1)
            for key in (sorted(value) if sort else value)
        ]
    else:
        raise TypeError(f"unexpected {type(value)}")
    if not items:
        return opening + closing
    if not pretty:
        return opening + ",".join(items) + closing
    inner = "  " * (depth + 1)
    return f"{opening}\n{inner}" + f",\n{inner}".join(items) + f"\n{'  ' * depth}{closing}"


def canonical(value, sort=True):
    return dump(value, sort=sort).encode()


def instant(text, where):
    """(UTC seconds, nanoseconds) of an RFC 3339 timestamp, ordered as chrono orders it."""
    match = TIMESTAMP.match(text) if isinstance(text, str) else None
    if not match:
        raise Refusal(f"{where}: unreadable timestamp {text!r}")
    fields = [int(part) for part in match.groups()[:6]]
    fraction, zone = match.group(7) or "", match.group(8)
    offset = 0 if zone == "Z" else int(zone[0] + "1") * (int(zone[1:3]) * 3600 + int(zone[4:6]) * 60)
    return calendar.timegm(tuple(fields)) - offset, int(fraction.ljust(9, "0"))


def normalize_artifact_path(path):
    """domain::artifact_plan::normalize_artifact_path."""
    components = []
    for component in path.replace("\\", "/").split("/"):
        if component in ("", "."):
            continue
        if component == ".." and components and components[-1] != "..":
            components.pop()
        else:
            components.append(component)
    return "/".join(components)


def evidence_key(target, where):
    """The key VirtualPath::archive_evidence_record gives a plan target."""
    kind = target.get("kind") if isinstance(target, dict) else None
    if kind == "container" and isinstance(target.get("id"), str):
        return f"container/{target['id']}"
    if kind == "document" and isinstance(target.get("path"), str):
        digest = hashlib.sha256(normalize_artifact_path(target["path"]).encode()).hexdigest()
        return f"document/{digest}"
    raise Refusal(f"{where}: unreadable archive target {target!r}")


def committed_lines(data):
    """The newline-terminated lines, and the uncommitted unterminated tail."""
    lines = data.split(b"\n")
    return [line + b"\n" for line in lines[:-1]], lines[-1]


class Run:
    def __init__(self, path, layout):
        self.path, self.layout = path, layout
        self.data = path.read_bytes()
        self.value = parse(self.data, path)
        if not isinstance(self.value, dict):
            raise Refusal(f"{path}: not a run record")
        self.issue, self.id = self.value.get("issue_id"), self.value.get("run_id")
        self.gate = self.value.get("gate_key")
        if not (isinstance(self.issue, str) and UUID.match(self.issue)):
            raise Refusal(f"{path}: unreadable issue id {self.issue!r}")
        if not (isinstance(self.id, str) and isinstance(self.gate, str)):
            raise Refusal(f"{path}: unreadable run id or gate key")
        self.started = instant(self.value.get("started_at"), path)
        self.origin = self.value.get("origin")
        if self.origin is not None and (
            not isinstance(self.origin, dict) or self.origin.get("derivation") not in ORIGINS
        ):
            raise Refusal(f"{path}: unreadable origin {self.origin!r}")

    def order(self):
        return self.started, self.id

    def reusable_digest(self):
        """GateRunResult::reusable_digest; a run with no origin executed its checker."""
        digest = self.value.get("inputs_digest")
        executed = self.origin is None or self.origin["derivation"] == "executed"
        if isinstance(digest, str) and executed and self.value.get("status") in REUSABLE_STATUSES:
            return digest
        return None


def read_runs(store):
    """Every run record under gate-runs/ in either layout, checked against its path."""
    root = store / "gate-runs"
    if not root.is_dir():
        return []
    runs = []
    for directory in sorted(root.iterdir()):
        if directory.name == "verdicts":
            continue
        if not (directory.is_dir() and UUID.match(directory.name)):
            raise Refusal(f"{directory}: not a run or issue directory")
        names = sorted(entry.name for entry in directory.iterdir())
        if "result.json" in names:
            if names != ["result.json"]:
                raise Refusal(f"{directory}: a format 3 run directory holding {names}")
            run = Run(directory / "result.json", "format3")
            if run.id != directory.name:
                raise Refusal(f"{run.path}: holds run id {run.id!r}")
            runs.append(run)
            continue
        for name in names:
            stem = name[: -len(".json")] if name.endswith(".json") else None
            if not (stem and UUID.match(stem) and (directory / name).is_file()):
                raise Refusal(f"{directory / name}: not a run record")
            run = Run(directory / name, "format4")
            if run.issue != directory.name or run.id != stem:
                raise Refusal(f"{run.path}: holds issue {run.issue!r} and run {run.id!r}")
            runs.append(run)
    return runs


def run_record_path(store, issue, run):
    return store / "gate-runs" / issue / f"{run}.json"


def verdict_path(store, gate, digest):
    return store / "gate-runs" / "verdicts" / gate / f"{digest}.json"


def listed_files(root, depth):
    """Files exactly `depth` directory levels below `root`."""
    if not root.is_dir():
        return set()
    return {path for path in root.glob("/".join(["*"] * (depth + 1))) if path.is_file()}


class Plan:
    """Every change, derived before any is made."""

    def __init__(self):
        self.writes = {}
        self.removals = []
        self.directories = []
        self.index = None
        self.summary = []

    def write(self, path, data):
        if not (path.is_file() and path.read_bytes() == data):
            self.writes[path] = data


def plan_index(store, plan):
    data = (store / "index.json").read_bytes()
    index = parse(data, store / "index.json")
    if not isinstance(index, dict) or set(index) != {"deleted_ids", "schema_version"}:
        raise Refusal(f"index.json holds fields {sorted(index) if isinstance(index, dict) else index!r}")
    version = int(index["schema_version"]) if isinstance(index["schema_version"], Number) else None
    if version == INDEX_VERSION:
        return
    if version != INDEX_VERSION - 1:
        raise Refusal(f"index.json schema_version is {index['schema_version']!r}")
    plan.index = canonical(dict(index, schema_version=Number(str(INDEX_VERSION))))


def plan_runs(store, plan):
    """The per-issue relayout and reused verdicts' source_issue; returns the runs by id."""
    by_id = {}
    for run in read_runs(store):
        by_id.setdefault(run.id, []).append(run)
    for run_id, copies in by_id.items():
        layouts = sorted(run.layout for run in copies)
        if layouts not in (["format3"], ["format4"], ["format3", "format4"]):
            raise Refusal(f"run {run_id} is recorded at {[str(run.path) for run in copies]}")
        if len({run.issue for run in copies}) != 1:
            raise Refusal(f"run {run_id} names different issues at {[str(run.path) for run in copies]}")
    # A run in both layouts is an interrupted earlier rewrite: the moved record is the run.
    runs = {run_id: max(copies, key=lambda run: run.layout) for run_id, copies in by_id.items()}

    def source_issue(run):
        source = runs.get(run.origin.get("source_run"))
        if source is None:
            raise Refusal(f"{run.path}: reused verdict names no recorded source run")
        stored = run.origin.get("source_issue")
        if stored is not None and stored != source.issue:
            raise Refusal(f"{run.path}: source_issue {stored!r} is not the source run's issue {source.issue!r}")
        return source.issue

    converted = sourced = 0
    for run_id, copies in by_id.items():
        target = run_record_path(store, copies[0].issue, run_id)
        for run in copies:
            data = run.data
            if run.origin is not None and run.origin["derivation"] == "reused":
                issue = source_issue(run)
                if "source_issue" not in run.origin:
                    data = with_source_issue(run, issue)
                    sourced += 1
            if run.layout == "format4":
                if data != run.data:
                    plan.write(target, data)
                continue
            twin = next((other for other in copies if other.layout == "format4"), None)
            if twin is not None and twin.data != data:
                raise Refusal(f"{run.path} and {twin.path} hold different records of run {run_id}")
            if twin is None:
                plan.write(target, data)
            plan.removals.append(run.path)
            plan.directories.append(run.path.parent)
            converted += 1
    plan.summary.append(f"moved {converted} of {len(runs)} run(s); source_issue on {sourced}")
    return runs


def with_source_issue(run, source_issue):
    """The run's bytes with source_issue inserted into its reused origin, nothing else changed."""
    text = run.data.decode()
    matches = list(REUSED_ORIGIN.finditer(text))
    if len(matches) != 1:
        raise Refusal(f"{run.path}: reused origin is not in the expected layout")
    at = matches[0].start(3)
    result = (text[:at] + f'"source_issue": "{source_issue}",\n{matches[0].group(2)}' + text[at:]).encode()
    expected = dict(run.value, origin=dict(run.value["origin"], source_issue=source_issue))
    if parse(result, run.path) != expected:
        raise Refusal(f"{run.path}: source_issue insertion changed other content")
    if canonical(run.value) == run.data and canonical(expected) != result:
        raise Refusal(f"{run.path}: source_issue insertion left a canonical record non-canonical")
    return result


def plan_gate_states(store, runs, plan):
    latest = {}
    for run in runs.values():
        key = (run.issue, run.gate)
        if key not in latest or run.order() > latest[key].order():
            latest[key] = run
    backfilled = issues_changed = 0
    for path in sorted((store / "issues").glob("*.json")):
        issue = parse(path.read_bytes(), path)
        states = issue.get("gates_status", {}) if isinstance(issue, dict) else None
        if not isinstance(states, dict):
            raise Refusal(f"{path}: unreadable gates_status")
        changed = False
        for gate, state in states.items():
            run = latest.get((issue.get("id"), gate))
            if run is None:
                continue
            if not isinstance(state, dict):
                raise Refusal(f"{path}: unreadable gate state {gate!r}")
            if "run_id" in state:
                if state["run_id"] != run.id:
                    raise Refusal(
                        f"{path}: gate {gate!r} names run {state['run_id']!r}, "
                        f"but its latest run is {run.id!r}"
                    )
                continue
            state["run_id"] = run.id
            changed = True
            backfilled += 1
        if changed:
            plan.write(path, canonical(issue))
            issues_changed += 1
    plan.summary.append(f"set run_id on {backfilled} gate state(s) in {issues_changed} issue(s)")
    return latest


def plan_verdicts(store, runs, plan):
    sources = {}
    for run in runs.values():
        digest = run.reusable_digest()
        key = (run.gate, digest)
        if digest is not None and (key not in sources or run.order() > sources[key].order()):
            sources[key] = run
    derived = {
        verdict_path(store, gate, digest): canonical({"issue_id": run.issue, "run_id": run.id})
        for (gate, digest), run in sources.items()
    }
    stray = listed_files(store / "gate-runs" / "verdicts", 1) - set(derived)
    if stray:
        raise Refusal(f"verdict records that no run derives: {sorted(map(str, stray))[:5]}")
    for path, data in derived.items():
        plan.write(path, data)
    plan.summary.append(f"{len(derived)} verdict record(s)")
    return derived


def plan_events(store, plan):
    """Archive evidence from the log, and the log without torn-tail markers."""
    path = store / "events.jsonl"
    data = path.read_bytes() if path.is_file() else b""
    lines, tail = committed_lines(data)
    evidence, rewritten, markers = {}, [], 0
    for number, line in enumerate(lines, 1):
        event = parse(line, f"{path}:{number}")
        kind = event.get("type") if isinstance(event, dict) else None
        if kind == "artifact_archive_executed":
            if not all(field in event for field in EXECUTION_FIELDS):
                raise Refusal(f"{path}:{number}: archive execution lacks a field")
            execution = {field: event[field] for field in EXECUTION_FIELDS}
            key = evidence_key(event["target"], f"{path}:{number}")
            evidence.setdefault(key, {"executions": [], "target": event["target"]})["executions"].append(execution)
        if kind == "profile_lifecycle" and "isolated_torn_tail" in event:
            if event["isolated_torn_tail"] is not False:
                raise Refusal(f"{path}:{number}: a torn-tail marker line carrying {event['isolated_torn_tail']!r}")
            body = line[:-1]
            found = [marker for marker in TORN_TAIL_MARKERS if body.count(marker) == 1]
            stripped = body.replace(found[0], b"", 1) + b"\n" if found else None
            expected = {key: value for key, value in event.items() if key != "isolated_torn_tail"}
            if stripped is None or parse(stripped, f"{path}:{number}") != expected:
                raise Refusal(f"{path}:{number}: torn-tail marker is not in the expected layout")
            line = stripped
            markers += 1
        rewritten.append(line)
    if markers:
        plan.write(path, b"".join(rewritten) + tail)
    derived = {
        store / "archive-evidence" / f"{key}.json": canonical(record) for key, record in evidence.items()
    }
    stray = listed_files(store / "archive-evidence", 1) - set(derived)
    if stray:
        raise Refusal(f"archive evidence that no logged execution derives: {sorted(map(str, stray))[:5]}")
    for evidence_path, record in derived.items():
        plan.write(evidence_path, record)
    plan.summary.append(f"removed {markers} torn-tail marker(s); {len(derived)} evidence record(s)")
    return derived


def plan_leases(shared, plan, now):
    """Lease records and claims.seq from claims.index.json; returns the derived lease paths."""
    index_path, log_path, seq_path = shared / "claims.index.json", shared / "claims.jsonl", shared / "claims.seq"
    heartbeat = shared / "heartbeat"
    if heartbeat.is_dir() and not any(heartbeat.iterdir()):
        plan.directories.append(heartbeat)
    if not index_path.is_file():
        if log_path.is_file() and not seq_path.is_file():
            raise Refusal(f"{log_path} has neither claims.index.json nor claims.seq beside it")
        plan.summary.append("no claims index")
        return None
    index = parse(index_path.read_bytes(), index_path)
    if not (isinstance(index, dict) and isinstance(index.get("leases"), list)):
        raise Refusal(f"{index_path}: unreadable lease index")
    log_seqs = [
        int(entry["seq"])
        for number, line in enumerate(committed_lines(log_path.read_bytes())[0] if log_path.is_file() else [], 1)
        for entry in [parse(line, f"{log_path}:{number}")]
        if isinstance(entry, dict) and isinstance(entry.get("seq"), Number)
    ]
    last_seq = index.get("last_seq")
    seq = max(log_seqs + ([int(last_seq)] if isinstance(last_seq, Number) else []) + [0])
    seq_bytes = str(seq).encode()
    if seq_path.is_file() and seq_path.read_bytes() != seq_bytes:
        raise Refusal(f"{seq_path} holds {seq_path.read_bytes()!r}; the claims derive {seq}")
    plan.write(seq_path, seq_bytes)
    derived, expired = {}, 0
    for lease in index["leases"]:
        if not (isinstance(lease, dict) and all(field in lease for field in LEASE_FIELDS)):
            raise Refusal(f"{index_path}: a lease lacks a field: {lease!r}")
        issue = lease["issue_id"]
        if not (isinstance(issue, str) and UUID.match(issue)):
            raise Refusal(f"{index_path}: unreadable lease issue {issue!r}")
        expires = lease["expires_at"]
        if int(lease["ttl_secs"]) > 0 and expires is not None and now > instant(expires, index_path):
            expired += 1
            continue
        path = shared / "leases" / f"{issue}.json"
        if path in derived:
            raise Refusal(f"{index_path}: two leases on issue {issue}")
        derived[path] = canonical({field: lease[field] for field in LEASE_FIELDS}, sort=False)
        if path.is_file() and path.read_bytes() != derived[path]:
            raise Refusal(f"{path} differs from the lease claims.index.json holds")
    stray = listed_files(shared / "leases", 0) - set(derived)
    stray = {path for path in stray if path.suffix == ".json"}
    if stray:
        raise Refusal(f"lease records beside claims.index.json: {sorted(map(str, stray))}")
    for path, data in derived.items():
        plan.write(path, data)
    plan.removals.append(index_path)
    plan.summary.append(f"{len(derived)} lease(s), {expired} expired dropped; claims.seq {seq}")
    return derived


def replace_file(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(f".{path.name}.format4-rewrite.tmp")
    with open(temp, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)


def apply(plan, store):
    for path, data in plan.writes.items():
        replace_file(path, data)
    for path in plan.removals:
        path.unlink(missing_ok=True)
    for directory in plan.directories:
        if directory.is_dir():
            directory.rmdir()
    if plan.index is not None:
        replace_file(store / "index.json", plan.index)


def verify(store, run_ids, latest, verdicts, evidence, leases, shared):
    """Read the result back: runs reachable per issue, and each record count as derived."""
    runs = read_runs(store)
    if any(run.layout != "format4" for run in runs):
        raise Refusal("a format 3 run directory remains")
    if sorted(run.id for run in runs) != sorted(run_ids):
        raise Refusal(f"run count changed: {len(run_ids)} before, {len(runs)} after")
    for run in runs:
        if run.path != run_record_path(store, run.issue, run.id):
            raise Refusal(f"{run.path} is not reachable from its issue's directory")
    unnamed = []
    for path in (store / "issues").glob("*.json"):
        issue = parse(path.read_bytes(), path)
        unnamed += [
            (path.name, gate)
            for gate, state in issue.get("gates_status", {}).items()
            if (issue["id"], gate) in latest and state.get("run_id") != latest[(issue["id"], gate)].id
        ]
    if unnamed:
        raise Refusal(f"gate states not naming their latest run: {unnamed[:5]}")
    for name, derived, found in (
        ("verdict", verdicts, listed_files(store / "gate-runs" / "verdicts", 1)),
        ("evidence", evidence, listed_files(store / "archive-evidence", 1)),
    ) + (
        (("lease", leases, {p for p in listed_files(shared / "leases", 0) if p.suffix == ".json"}),)
        if leases is not None
        else ()
    ):
        if found != set(derived) or any(path.read_bytes() != data for path, data in derived.items()):
            raise Refusal(f"{len(found)} {name} record(s) on disk; derived {len(derived)}")
    index = parse((store / "index.json").read_bytes(), "index.json")
    if int(index["schema_version"]) != INDEX_VERSION:
        raise Refusal("index.json is not at format 4")


def shared_jit(checkout):
    """<common-dir>/jit, which WorktreePaths::shared_jit names, or None outside git."""
    result = subprocess.run(
        ["git", "-C", str(checkout), "rev-parse", "--path-format=absolute", "--git-common-dir"],
        capture_output=True,
        text=True,
    )
    return Path(result.stdout.strip()) / "jit" if result.returncode == 0 else None


def rewrite(checkout, now):
    store = Path(checkout) / ".jit"
    if not (store / "index.json").is_file():
        raise Refusal(f"no index at {store / 'index.json'}")
    shared = shared_jit(checkout)
    plan = Plan()
    plan_index(store, plan)
    runs = plan_runs(store, plan)
    latest = plan_gate_states(store, runs, plan)
    verdicts = plan_verdicts(store, runs, plan)
    evidence = plan_events(store, plan)
    leases = plan_leases(shared, plan, now) if shared is not None and shared.is_dir() else None
    apply(plan, store)
    verify(store, list(runs), latest, verdicts, evidence, leases, shared)
    state = "rewritten to format 4" if plan.index is not None else "already format 4"
    changed = len(plan.writes) + len(plan.removals) + (plan.index is not None)
    return f"{store}: index {state}; {'; '.join(plan.summary)}; {changed} file change(s)"


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: format4-rewrite.py <checkout>")
    now = datetime.now(timezone.utc)
    try:
        print(rewrite(sys.argv[1], (calendar.timegm(now.timetuple()), now.microsecond * 1000)))
    except Refusal as refusal:
        sys.exit(f"format4-rewrite: refused: {refusal}")


if __name__ == "__main__":
    main()
