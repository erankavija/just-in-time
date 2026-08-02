<!-- Generated from `crate::storage::reference` — do not edit by hand. -->

# Storage Record Layout

> **Diátaxis Type:** Reference

The shape of an issue identifier, of a line in the event log, and of a recorded
gate run. This page is generated from the definitions the binary writes with —
`crates/jit/src/domain/types.rs` and `crates/jit/src/storage/` — so its widths,
paths, and field lists are the ones in force. For the `.jit/` directory as a
whole, its configuration and registry files, and the issue record's own field
table, see [Storage Format](storage-format.md).

## Issue Identifiers

An issue's `id` is a UUID v4, stored in the canonical hyphenated form
(`9d1f6c02-4a77-4f2b-8f3d-5e0b7a1c8e64`). Creation mints it, the issue's record
lives at `.jit/issues/<id>.json`, and every stored cross-reference — a
`dependencies` entry, an event's `issue_id`, a gate run's `issue_id` — carries
this full id.

The **short id** the CLI prints is the first 8 characters of that
string. It is a human-facing convention computed on read, not a stored field: no
record carries it, and nothing enforces its uniqueness — two issues whose UUIDs
share their leading 8 characters would print the same short id.

Commands take an id **prefix** wherever they take an issue id. Resolution
lowercases the input and drops its hyphens to measure it, then:

- an input of full-id length (32 hex digits once normalized) is looked up directly, as given: it resolves only in the canonical hyphenated lowercase form the record is stored under, and is not searched for as a prefix;
- a shorter input must be at least 4 characters after normalization; below the minimum it is refused as an argument error (exit code 2, see [Exit Codes](exit-codes.md)) rather than searched for;
- the prefix must match exactly one id in the repository index. Several matches are refused as ambiguous, and the error lists the candidates.

A short id is 8 characters and the minimum is 4,
so a short id printed by one command is always long enough to hand back to the next.

## Event Log Records

`.jit/events.jsonl` is the event log, in JSON Lines: one JSON object per line,
each terminated by a newline. Appending a record serializes it to a single line
and writes it at the end of the file under an exclusive lock; records are never
rewritten in place, and reading parses the file line by line, skipping blank
lines. Every record carries a `type` tag, its own `id`, and a `timestamp`; the
remaining fields are flat on the object and vary by tag.

```jsonl
{"type":"issue_created","id":"00000000-0000-0000-0000-000000000000","issue_id":"00000000-0000-0000-0000-000000000001","timestamp":"1970-01-01T00:00:00Z","title":"Sample issue","priority":"normal"}
{"type":"issue_state_changed","id":"00000000-0000-0000-0000-000000000000","issue_id":"00000000-0000-0000-0000-000000000001","timestamp":"1970-01-01T00:00:00Z","from":"ready","to":"in_progress"}
{"type":"gate_definition_created","id":"00000000-0000-0000-0000-000000000000","timestamp":"1970-01-01T00:00:00Z","gate_key":"tests"}
```

The tag vocabulary — every `type` value, what appends it, and which tags carry an
`issue_id` — is the generated [Event Log Tags](events.md) reference, which this
page does not restate.

## Gate Run Records

Running a gate records its result at `.jit/gate-runs/<run-id>/result.json`.
The `<run-id>` is a UUID v4 minted per execution, so runs accumulate: a rerun
writes a new directory beside the old one, and the history of a gate on an issue
is the set of run directories whose record names that issue. The record is
written as pretty-printed JSON in the same recoverable repository transaction as
its coupled issue and event updates.

```json
{
  "schema_version": 1,
  "run_id": "7b2b0a4c-2f5a-4d3e-9b1a-6c8f0d5e4a21",
  "gate_key": "tests",
  "stage": "postcheck",
  "issue_id": "9d1f6c02-4a77-4f2b-8f3d-5e0b7a1c8e64",
  "commit": "9f1c0f0a2b3c4d5e6f708192a3b4c5d6e7f80910",
  "branch": "main",
  "tree_dirty": false,
  "status": "passed",
  "started_at": "2026-01-09T23:06:40Z",
  "completed_at": "2026-01-09T23:06:40Z",
  "duration_ms": 12480,
  "exit_code": 0,
  "stdout": "test result: ok. 812 passed; 0 failed",
  "stderr": "",
  "command": "cargo test --workspace",
  "by": "agent:worker-1",
  "message": "all suites green",
  "findings": {
    "verdict": "pass",
    "summary": "no blocking findings",
    "findings": [
      {
        "id": "F1",
        "severity": "low",
        "disposition": "advisory",
        "origin": "issue-impact",
        "summary": "the temp path deserves a name",
        "file": "crates/jit/src/storage/json.rs",
        "line": 793,
        "references": [
          "@/inv/atomic-writes"
        ]
      }
    ]
  },
  "inputs_digest": "105438fba9bc6384d757de31311105b5ea4839a5d519b14fcf59af7307567ef4",
  "origin": {
    "derivation": "reused",
    "source_run": "1c4e8a90-3d2b-4f61-9a07-5b8c2d1e6f34"
  }
}
```

The example above carries every field. The **presence** column below says what
the encoding does with a field that has no value: `always` fields are written on
every record, `null when unset` fields keep their key, and `omitted when unset`
fields drop out of the object entirely — so a reader must treat an absent key and
a `null` one alike.

| Field | Presence | Meaning |
| --- | --- | --- |
| `schema_version` | always | Record-format version of this run record; this binary writes `1`. It versions the run record alone, independently of the repository format version in `index.json`. |
| `run_id` | always | The run's identifier, and the name of its directory under `gate-runs/`. Each execution mints a fresh UUID v4, so a rerun records a new directory instead of overwriting the previous run. |
| `gate_key` | always | Key of the gate that ran, as registered in `.jit/gates.toml`. |
| `stage` | always | Stage the gate ran at: `precheck` or `postcheck`. |
| `issue_id` | always | Full id of the issue the run is about. An issue's runs are selected by matching this field across the run directories, so `gate-runs/` is flat rather than nested per issue. |
| `commit` | `null` when unset | Git commit the checker was launched at, when the working directory is a git repository. Unset for a run that launched no checker: a verdict taken from an earlier run ran at no commit of its own, and `origin` names the run that carries one. |
| `branch` | `null` when unset | Git branch the checker was launched on, under the same conditions as `commit`. |
| `tree_dirty` | `null` when unset | Whether the working tree differed from `commit` when the checker started: `true` if it carried uncommitted or untracked changes, `false` if it matched the commit exactly. A `true` run evidences that modified tree rather than the commit alone. `null` when no cleanliness value provably describes the recorded commit: there was no commit to compare against (not a git repository, no commits yet, or no checker launched), or `HEAD` moved through every paired probe attempt while the evidence was being taken, so the tree state is recorded as unknown rather than paired with a commit it might not describe. |
| `status` | always | The run's verdict: `passed`, `failed`, `error`, `pending`, or `skipped`. For an executed checker the exit code decides it — `0` passes; a shell that could not run the command (`126`, `127`) and a checker killed by a signal or by its timeout are an `error`; any other code fails. |
| `started_at` | always | RFC 3339 timestamp taken before the checker is launched. |
| `completed_at` | `null` when unset | RFC 3339 timestamp taken once the checker has returned. |
| `duration_ms` | `null` when unset | Wall-clock duration of the checker, in milliseconds. |
| `exit_code` | `null` when unset | Exit code the checker returned; unset when it was killed by a signal or by its timeout. |
| `stdout` | always | The checker's captured standard output, kept verbatim — including any findings block, which `findings` carries in parsed form. |
| `stderr` | always | The checker's captured standard error, kept verbatim. |
| `command` | always | The command line that was executed. |
| `by` | `null` when unset | Who triggered the run. |
| `message` | `null` when unset | Free-text note attached to the run. |
| `findings` | omitted when unset | Structured findings parsed from the checker's machine-readable block, carrying the checker's `verdict`, a `summary`, and the `findings` array. Each finding may carry an optional `references` array of opaque strings; it is omitted when empty. Unset when the checker emitted no such block; the raw `stdout` is kept either way. `jit gate status --findings` prints this field. |
| `inputs_digest` | omitted when unset | Digest of the repository files the gate declares its checker reads, taken before the verdict was obtained. Two runs of one gate carrying the same value read byte-identical content at byte-identical paths. Unset when the gate declares no inputs, which is what keeps its checker running on every evaluation. |
| `origin` | always | Where the verdict came from. `{"derivation": "executed"}` means this run ran the checker; `{"derivation": "reused", "source_run": "<run-id>"}` means it carries the named earlier run's verdict, whose `inputs_digest` matched, without the checker running again. The report text behind a reused verdict lives at the named run. |
