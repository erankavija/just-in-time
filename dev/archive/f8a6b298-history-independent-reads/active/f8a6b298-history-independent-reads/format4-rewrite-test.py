#!/usr/bin/env python3
"""Tests for format4-rewrite.py over synthetic format 3 checkouts.

Usage: python3 -I format4-rewrite-test.py
"""
import hashlib
import importlib.util
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("rewrite", Path(__file__).with_name("format4-rewrite.py"))
rewrite = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rewrite)
# Scratch checkouts live in the worktree's target/, never the system temp directory.
SCRATCH = Path(__file__).resolve().parents[3] / "target" / "format4-rewrite-test"

NOW = rewrite.instant("2026-11-04T00:00:00Z", "test")
ISSUE_A = "aaaaaaaa-0000-4000-8000-000000000001"
ISSUE_B = "bbbbbbbb-0000-4000-8000-000000000002"
DIGEST = "d" * 64
RUN_OLD = "00000000-0000-4000-8000-0000000000a1"
RUN_NEW = "00000000-0000-4000-8000-0000000000a2"
RUN_REUSED = "00000000-0000-4000-8000-0000000000a3"
RUN_LEGACY = "00000000-0000-4000-8000-0000000000a4"
CONTAINER = {"id": ISSUE_A, "kind": "container"}
DOCUMENT = {"kind": "document", "path": "./dev/notes.md"}


def pretty(value, sort=True):
    return json.dumps(value, indent=2, sort_keys=sort, ensure_ascii=False).encode()


def compact(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode() + b"\n"


def run_record(run_id, issue, started, status="passed", origin=None, digest=DIGEST):
    record = {
        "branch": "main",
        "by": "auto:executor",
        "command": "true",
        "commit": "c" * 40,
        "completed_at": started,
        "duration_ms": 5,
        "evaluation_stability": "stable",
        "exit_code": 0,
        "gate_key": "tests",
        "inputs_digest": digest,
        "issue_id": issue,
        "message": None,
        "origin": origin or {"derivation": "executed"},
        "run_id": run_id,
        "schema_version": 1,
        "stage": "postcheck",
        "started_at": started,
        "status": status,
        "stderr": "",
        "stdout": "ok ✓\n",
    }
    return record


def issue_record(issue, run_id=None):
    state = {"status": "passed", "updated_at": "2026-11-01T00:00:00Z", "updated_by": "auto:executor"}
    if run_id is not None:
        state["run_id"] = run_id
    return {
        "dependencies": [],
        "description": "",
        "gates_required": ["tests"],
        "gates_status": {"tests": state},
        "id": issue,
        "labels": [],
        "title": f"issue {issue[:8]}",
    }


def archive_event(event_id, target):
    return {
        "destination_root": "dev/archive",
        "id": event_id,
        "planned_deletions": [],
        "publications": [],
        "reconciling": False,
        "reference_changes": [],
        "target": target,
        "timestamp": "2026-11-01T00:00:00Z",
        "type": "artifact_archive_executed",
    }


def lease(issue, ttl, expires, beat="2026-11-03T00:00:00Z"):
    return {
        "lease_id": f"{issue[:-4]}ffff",
        "issue_id": issue,
        "agent_id": "agent:worker",
        "worktree_id": "wt:00000001",
        "branch": "main",
        "ttl_secs": ttl,
        "acquired_at": "2026-11-03T00:00:00Z",
        "expires_at": expires,
        "last_beat": beat,
        "stale": ttl == 0,
    }


ACTIVE = "cccccccc-0000-4000-8000-000000000003"
STALE = "dddddddd-0000-4000-8000-000000000004"
EXPIRED = "eeeeeeee-0000-4000-8000-000000000005"
MARKER_LINE = {
    "id": "e3",
    "isolated_torn_tail": False,
    "operation": "apply",
    "profiles": [],
    "timestamp": "2026-11-01T00:00:00Z",
    "type": "profile_lifecycle",
}


def build_checkout(root):
    """A format 3 checkout holding one instance of each convertible shape."""
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    store = root / ".jit"
    (store / "issues").mkdir(parents=True)
    (store / "index.json").write_bytes(pretty({"deleted_ids": [], "schema_version": 3}))
    (store / "issues" / f"{ISSUE_A}.json").write_bytes(pretty(issue_record(ISSUE_A)))
    (store / "issues" / f"{ISSUE_B}.json").write_bytes(pretty(issue_record(ISSUE_B, RUN_REUSED)))
    runs = {
        RUN_OLD: pretty(run_record(RUN_OLD, ISSUE_A, "2026-11-01T00:00:00.5Z")),
        RUN_NEW: pretty(run_record(RUN_NEW, ISSUE_A, "2026-11-01T01:00:00.123456789+02:00", "failed")),
        RUN_REUSED: pretty(
            run_record(
                RUN_REUSED,
                ISSUE_B,
                "2026-11-02T00:00:00Z",
                origin={"derivation": "reused", "source_run": RUN_NEW},
            )
        ),
        # An earlier key set in struct order: moved as stored, not reformatted.
        RUN_LEGACY: pretty(
            {
                key: value
                for key, value in run_record(RUN_LEGACY, ISSUE_A, "2026-10-01T00:00:00+02:00").items()
                if key not in ("origin", "evaluation_stability", "inputs_digest")
            },
            sort=False,
        ),
    }
    for run_id, data in runs.items():
        (store / "gate-runs" / run_id).mkdir(parents=True)
        (store / "gate-runs" / run_id / "result.json").write_bytes(data)
    events = [
        compact(archive_event("e1", CONTAINER)),
        compact({"id": "e2", "issue_id": ISSUE_A, "timestamp": "2026-11-01T00:00:00Z", "type": "issue_created"}),
        compact(MARKER_LINE),
        compact(archive_event("e4", DOCUMENT)),
        compact(archive_event("e5", CONTAINER)),
    ]
    (store / "events.jsonl").write_bytes(b"".join(events) + b'{"torn')
    shared = root / ".git" / "jit"
    (shared / "heartbeat").mkdir(parents=True)
    (shared / "locks").mkdir()
    index = {
        "schema_version": 1,
        "generated_at": "2026-11-03T00:00:00Z",
        "last_seq": 7,
        "sequence_gaps": [],
        "leases": [
            lease(ACTIVE, 86400, "2026-11-05T00:00:00Z"),
            lease(STALE, 0, None, beat="2026-10-01T00:00:00Z"),
            lease(EXPIRED, 60, "2026-11-03T00:01:00Z"),
        ],
    }
    (shared / "claims.index.json").write_bytes(pretty(index, sort=False))
    (shared / "claims.jsonl").write_bytes(b'{"op":"acquire","seq":8}\n{"op":"release","seq":6}\n')
    return runs, events


def snapshot(root):
    return {
        str(path.relative_to(root)): path.read_bytes() if path.is_file() else None
        for path in sorted(root.rglob("*"))
        if ".git/jit" in str(path) or ".jit" in path.parts
    }


class RewriteTest(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(dir=SCRATCH))
        self.runs, self.events = build_checkout(self.root)
        self.store, self.shared = self.root / ".jit", self.root / ".git" / "jit"

    def tearDown(self):
        shutil.rmtree(self.root)

    def convert(self):
        return rewrite.rewrite(self.root, NOW)

    def assert_refused_unchanged(self, fragment):
        before = snapshot(self.root)
        with self.assertRaises(rewrite.Refusal) as refusal:
            self.convert()
        self.assertIn(fragment, str(refusal.exception))
        self.assertEqual(snapshot(self.root), before)

    def test_rewrite_sets_schema_version_4_in_canonical_bytes(self):
        self.convert()
        self.assertEqual(
            (self.store / "index.json").read_bytes(), pretty({"deleted_ids": [], "schema_version": 4})
        )

    def test_rewrite_moves_each_run_under_its_issue_with_other_bytes_unchanged(self):
        self.convert()
        self.assertEqual(sorted(p.name for p in (self.store / "gate-runs").iterdir()), sorted([ISSUE_A, ISSUE_B, "verdicts"]))
        for run_id, data in self.runs.items():
            issue = ISSUE_B if run_id == RUN_REUSED else ISSUE_A
            moved = (self.store / "gate-runs" / issue / f"{run_id}.json").read_bytes()
            if run_id == RUN_REUSED:
                self.assertEqual(moved.replace(f'    "source_issue": "{ISSUE_A}",\n'.encode(), b""), data)
            else:
                self.assertEqual(moved, data)

    def test_rewrite_gives_a_reused_verdict_its_source_issue_in_canonical_form(self):
        self.convert()
        moved = (self.store / "gate-runs" / ISSUE_B / f"{RUN_REUSED}.json").read_bytes()
        expected = run_record(
            RUN_REUSED,
            ISSUE_B,
            "2026-11-02T00:00:00Z",
            origin={"derivation": "reused", "source_issue": ISSUE_A, "source_run": RUN_NEW},
        )
        self.assertEqual(moved, pretty(expected))

    def test_rewrite_sets_run_id_from_the_greatest_started_at_and_run_id(self):
        self.convert()
        issue = json.loads((self.store / "issues" / f"{ISSUE_A}.json").read_bytes())
        # RUN_OLD starts later than RUN_NEW, whose id and timestamp text sort greater.
        self.assertEqual(issue["gates_status"]["tests"]["run_id"], RUN_OLD)

    def test_rewrite_names_the_latest_qualifying_run_in_each_verdict_record(self):
        self.convert()
        records = list((self.store / "gate-runs" / "verdicts").rglob("*.json"))
        self.assertEqual(records, [self.store / "gate-runs" / "verdicts" / "tests" / f"{DIGEST}.json"])
        # The reused run is later but did not execute its checker.
        self.assertEqual(records[0].read_bytes(), pretty({"issue_id": ISSUE_A, "run_id": RUN_OLD}))

    def test_rewrite_writes_one_evidence_record_per_target_in_log_order(self):
        self.convert()
        container = json.loads((self.store / "archive-evidence" / "container" / f"{ISSUE_A}.json").read_bytes())
        digest = hashlib.sha256(b"dev/notes.md").hexdigest()
        document = json.loads((self.store / "archive-evidence" / "document" / f"{digest}.json").read_bytes())
        self.assertEqual([len(container["executions"]), len(document["executions"])], [2, 1])
        self.assertEqual(container["target"], CONTAINER)
        self.assertNotIn("id", container["executions"][0])

    def test_rewrite_removes_the_marker_and_keeps_other_lines_byte_identical(self):
        self.convert()
        lines = (self.store / "events.jsonl").read_bytes().split(b"\n")
        expected = [line.rstrip(b"\n") for line in self.events]
        expected[2] = compact({k: v for k, v in MARKER_LINE.items() if k != "isolated_torn_tail"}).rstrip(b"\n")
        self.assertEqual(lines, expected + [b'{"torn'])

    def test_rewrite_writes_each_unexpired_lease_including_a_stale_one(self):
        self.convert()
        records = sorted(p.name for p in (self.shared / "leases").iterdir())
        self.assertEqual(records, sorted([f"{ACTIVE}.json", f"{STALE}.json"]))
        stored = (self.shared / "leases" / f"{STALE}.json").read_bytes()
        record = lease(STALE, 0, None, beat="2026-10-01T00:00:00Z")
        del record["stale"]
        self.assertEqual(stored, pretty(record, sort=False))

    def test_rewrite_seeds_claims_seq_from_the_greater_of_log_and_index(self):
        self.convert()
        self.assertEqual((self.shared / "claims.seq").read_bytes(), b"8")

    def test_rewrite_removes_the_claims_index_and_empty_heartbeat_keeping_the_log(self):
        log = (self.shared / "claims.jsonl").read_bytes()
        self.convert()
        self.assertEqual(sorted(p.name for p in self.shared.iterdir()), ["claims.jsonl", "claims.seq", "leases", "locks"])
        self.assertEqual((self.shared / "claims.jsonl").read_bytes(), log)

    def test_rewrite_second_run_changes_no_byte(self):
        self.convert()
        before = snapshot(self.root)
        self.convert()
        self.assertEqual(snapshot(self.root), before)

    def test_rewrite_over_a_merged_format3_branch_converts_only_what_it_added(self):
        self.convert()
        before = snapshot(self.root)
        added_run = "00000000-0000-4000-8000-0000000000b1"
        added_issue = "ffffffff-0000-4000-8000-000000000006"
        data = pretty(run_record(added_run, added_issue, "2026-11-03T00:00:00Z", digest="e" * 64))
        (self.store / "gate-runs" / added_run).mkdir()
        (self.store / "gate-runs" / added_run / "result.json").write_bytes(data)
        (self.store / "issues" / f"{added_issue}.json").write_bytes(pretty(issue_record(added_issue)))
        self.convert()
        after = snapshot(self.root)
        changed = {path for path in after if path not in before or before[path] != after[path]} | (set(before) - set(after))
        self.assertEqual(
            changed,
            {
                f".jit/gate-runs/{added_issue}",
                f".jit/gate-runs/{added_issue}/{added_run}.json",
                f".jit/issues/{added_issue}.json",
                ".jit/gate-runs/verdicts/tests/" + "e" * 64 + ".json",
            },
        )
        self.assertEqual(after[f".jit/gate-runs/{added_issue}/{added_run}.json"], data)

    def test_rewrite_refuses_a_reused_verdict_with_no_resolvable_source(self):
        shutil.rmtree(self.store / "gate-runs" / RUN_NEW)
        self.assert_refused_unchanged("names no recorded source run")

    def test_rewrite_refuses_a_stored_run_id_that_is_not_the_greatest_run(self):
        (self.store / "issues" / f"{ISSUE_A}.json").write_bytes(pretty(issue_record(ISSUE_A, RUN_NEW)))
        self.assert_refused_unchanged("its latest run is")

    def test_rewrite_refuses_a_format3_run_directory_naming_another_run(self):
        (self.store / "gate-runs" / RUN_OLD).rename(self.store / "gate-runs" / "00000000-0000-4000-8000-0000000000c1")
        self.assert_refused_unchanged("holds run id")

    def test_rewrite_refuses_a_format4_run_record_under_another_issue(self):
        self.convert()
        (self.store / "gate-runs" / ISSUE_A / f"{RUN_OLD}.json").rename(
            self.store / "gate-runs" / ISSUE_B / f"{RUN_OLD}.json"
        )
        self.assert_refused_unchanged("holds issue")

    def test_rewrite_refuses_a_marker_line_carrying_true(self):
        marked = compact(dict(MARKER_LINE, isolated_torn_tail=True))
        log = b"".join(self.events).replace(self.events[2], marked)
        (self.store / "events.jsonl").write_bytes(log)
        self.assert_refused_unchanged("carrying True")


if __name__ == "__main__":
    unittest.main()
