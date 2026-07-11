# 8682e95a — Command-family reference gap-fill notes

Two additive sections added to `docs/reference/cli-commands.md` for the two CLI
families that lacked a reference home. Every documented flag/subcommand verified
against the current binary (`jit 0.2.1`, commit `b906051a`, matches HEAD).

## REQ-01 — `serve` family

Placed as a new `## Server Commands` top-level section, between `## Snapshot
Commands` and `## Git Hook Commands` (the operational/runtime cluster).

Source of truth: `jit --schema | jq '.commands.serve'` (flag names + descriptions)
and `crates/jit/src/cli.rs:427-455` (defaults, conflicts). Behaviour/output shapes
from `crates/jit/src/commands/serve.rs:1-6` (PID file `.jit/server.pid.json`,
port range 3000–3099) and the dispatch in `crates/jit/src/main.rs:6345-6575`
(JSON status objects: `started`/`running`/`stopped`/`not_running`/`exited`).

Flags documented (all in `--schema`):

- `--port <PORT>` — cli.rs:429 `default_value = "3000"`, `u16`; auto-selects 3000–3099 when taken.
- `--stop` — cli.rs:433, `conflicts_with_all = ["status", "fg"]`.
- `--status` — cli.rs:437, `conflicts_with_all = ["stop", "fg"]`.
- `--fg` — cli.rs:441.
- `--log <FILE>` — cli.rs:445, `Option<String>`, resolved under `.jit/` (main.rs:6360 `jit_dir.join(l)`), default `.jit/server.log`.
- `--web-dir <DIR>` — cli.rs:449, auto-detected when omitted.
- `--json` — cli.rs:453.

Mutual exclusion of `--stop`/`--status`/`--fg` documented from the clap
`conflicts_with_all` attrs. No short forms exist for this family.

## REQ-02 — `events` family

Placed as a new `## Event Log Commands` top-level section, between `## Query
Commands` and `## Document Commands` (read/inspection surface over the log).

Source of truth: `jit --schema | jq '.commands.events'` and
`crates/jit/src/cli.rs:2132-2161` (short forms + defaults, which `--schema`
omits). Output rendering from `crates/jit/src/main.rs:4328-4372`: human output is
JSONL (one event object per line); `--json` is the list envelope
`{"count": N, "events": [...]}` + `message`.

The event object shape and the full set of event `type` tags are NOT hand-copied
(single-source-prose, `@/inv/single-source-prose`); the section cites
`storage-format.md#event-log-format` for both.

`events tail` flags:

- `-n <N>` — cli.rs:2138 `#[arg(short, long, default_value = "10")]` on field `n` (short `-n`; long form is `--n`, not documented to avoid clutter — `-n` is the primary form).
- `--json` — cli.rs:2141.

`events query` flags:

- `-e`, `--event-type <TYPE>` — cli.rs:2149 `#[arg(short, long)]` on `event_type`.
- `-i`, `--issue-id <ID>` — cli.rs:2152 `#[arg(short, long)]` on `issue_id`.
- `-l`, `--limit <N>` — cli.rs:2155 `#[arg(short, long, default_value = "50")]` on `limit`.
- `--json` — cli.rs:2158.

## Self-verification (over the footprint)

- `./scripts/docs-mechanical.sh docs/reference/cli-commands.md` → EXIT 0 (M2 links/anchors, M3 citations, M5 projections all OK).
- M1 invented-flag guard: no residue attributable to the new sections; every added `--flag` resolves to `--schema`. Whole-file residue is the expected set (cargo `--lib`/`--workspace`, globals `--help`/`--schema`/`--version`/`--quiet`, alias `--add-label`, `--clear-` truncation).
- M4 box-drawing: no box-drawing characters in the file.

## Judgment calls

- Placement: `serve` → new `## Server Commands` in the operational cluster;
  `events` → new `## Event Log Commands` after Query Commands. Both are net-new
  top-level families with no prior home, so a new section per family (not an entry
  under an existing family) is the native shape.
- M3 citation guard initially flagged `.jit/server.log` and `.jit/server.pid.json`
  (gitignored machine-local files that do not exist on disk; `.jit` is a tracked
  root). Reworded to the established `storage-format.md:36` convention — keep the
  bare filename in the backtick span and `.jit/` as a separate trailing-slash dir
  token, both of which M3 defers — rather than asserting a path that will never
  exist in the tree.
