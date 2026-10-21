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
each terminated by a newline. A record is serialized to a single line with its
object keys sorted, and a mutation appends its lines at the log length it read;
records are never rewritten in place. The publication contract is
[Publication and Recovery](storage-format.md#publication-and-recovery). Reading
parses the file line by line, skipping blank lines. Every record carries a
`type` tag, its own `id`, and a `timestamp`; the remaining fields are flat on
the object and vary by tag.

```jsonl
{"id":"00000000-0000-0000-0000-000000000000","issue_id":"00000000-0000-0000-0000-000000000001","priority":"normal","timestamp":"1970-01-01T00:00:00Z","title":"Sample issue","type":"issue_created"}
{"from":"ready","id":"00000000-0000-0000-0000-000000000000","issue_id":"00000000-0000-0000-0000-000000000001","timestamp":"1970-01-01T00:00:00Z","to":"in_progress","type":"issue_state_changed"}
{"gate_key":"tests","id":"00000000-0000-0000-0000-000000000000","timestamp":"1970-01-01T00:00:00Z","type":"gate_definition_created"}
```

The tag vocabulary — every `type` value, what appends it, and which tags carry an
`issue_id` — is the generated [Event Log Tags](events.md) reference, which this
page does not restate.

## Gate Run Records

Running a gate records its result at `.jit/gate-runs/<run-id>/result.json`.
The `<run-id>` is a UUID v4 minted per execution, so runs accumulate: a rerun
writes a new directory beside the old one, and the history of a gate on an issue
is the set of run directories whose record names that issue. The record is
written as pretty-printed JSON in the same redo record as
its coupled issue and event updates.

```json
{
  "branch": "main",
  "by": "agent:worker-1",
  "command": "cargo test --workspace",
  "commit": "9f1c0f0a2b3c4d5e6f708192a3b4c5d6e7f80910",
  "completed_at": "2026-01-09T23:06:40Z",
  "duration_ms": 12480,
  "evaluation_stability": "inputs_moved",
  "exit_code": 0,
  "findings": {
    "findings": [
      {
        "disposition": "advisory",
        "file": "crates/jit/src/storage/json.rs",
        "id": "F1",
        "line": 793,
        "origin": "issue-impact",
        "references": [
          "@/inv/atomic-writes"
        ],
        "severity": "low",
        "summary": "the temp path deserves a name"
      }
    ],
    "summary": "no blocking findings",
    "verdict": "pass"
  },
  "gate_key": "tests",
  "inputs_digest": "105438fba9bc6384d757de31311105b5ea4839a5d519b14fcf59af7307567ef4",
  "issue_id": "9d1f6c02-4a77-4f2b-8f3d-5e0b7a1c8e64",
  "message": "all suites green",
  "origin": {
    "derivation": "reused",
    "source_run": "1c4e8a90-3d2b-4f61-9a07-5b8c2d1e6f34"
  },
  "run_id": "7b2b0a4c-2f5a-4d3e-9b1a-6c8f0d5e4a21",
  "schema_version": 1,
  "stage": "postcheck",
  "started_at": "2026-01-09T23:06:40Z",
  "status": "passed",
  "stderr": "",
  "stdout": "test result: ok. 812 passed; 0 failed"
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
| `status` | always | The run's verdict: `passed`, `failed`, `error`, `pending`, or `skipped`. For an executed checker the exit status decides it, as [Checker Exit Codes](../how-to/custom-gates.md#exit-codes) defines. |
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
| `evaluation_stability` | always | Whether the evaluation inputs still matched the repository when the run was recorded. `stable` means they held still; `inputs_moved` means the verdict is the checker's verdict over the state captured before it started, not over the state the record was published against. A reader can therefore distinguish post-run repository drift without treating the completed verdict as discarded. |
