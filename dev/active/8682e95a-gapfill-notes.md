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

- `--port <PORT>` — cli.rs:429 `default_value = "3000"`, `u16`; when taken, `find_available_port` (serve.rs:238-258) scans `start..=start+99` — the 100 ports from the REQUESTED port upward (3000–3099 for the default; 5000–5099 for `--port 5000`), NOT a fixed 3000–3099.
- `--stop` — cli.rs:433, `conflicts_with_all = ["status", "fg"]`.
- `--status` — cli.rs:437, `conflicts_with_all = ["stop", "fg"]`.
- `--fg` — cli.rs:441.
- `--log <FILE>` — cli.rs:445, `Option<String>`; main.rs:6360 `jit_dir.join(l)`. `Path::join` keeps an absolute argument absolute, so a RELATIVE path resolves under `.jit/` (default `server.log` there) while an ABSOLUTE path is used as given.
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

## Rework attempt 1 (doc-review: 3 serve behavioral drifts)

All three verified against source and corrected; events section untouched.

- F1 [high] port range — was "auto-selects a free port in the range 3000–3099"
  (fixed range). `find_available_port` (serve.rs:238-258) scans `start..=start+99`
  RELATIVE to the requested port. Fixed in BOTH the prose (the range is only
  3000–3099 for the default) and the `--port` flag-table row (the lead cited only
  the prose; the flag row carried the same fixed-range claim and was corrected too).
- F2 [medium] `--log` absolute paths — was "resolved under `.jit/`" flat.
  main.rs:6360 `jit_dir.join(l)`; `Path::join` leaves an absolute argument
  absolute. Flag row now: relative → under `.jit/`; absolute → used as given.
- F3 [medium] status taxonomy — added `error` (emitted by the serve handler on a
  `--stop`/`--status` failure as `{"status":"error","error":<msg>}`), which the
  documented `status` value list had omitted.

Behavioral sweep of the whole serve section (each claim re-verified, no further
drift): background-daemon start (start_server daemonizes); PID file
`.jit/server.pid.json` (serve.rs:3); already-running dedup (ServeOutcome::
AlreadyRunning); `/api` + `/` endpoints and filesystem-vs-embedded UI (main.rs
prints; web_ui_source); `--stop`/`--status`/`--fg` mutual exclusion (clap
conflicts_with_all); `--web-dir` auto-detect (find_web_dir); `--json` started
shape `{status,pid,port,url,log_file,web_ui,web_ui_source}` (main.rs:6542-6564,
web_ui hardcoded true on start). All match source.

## Rework attempt 2 (doc-review: 2 mode-conditional edge cases)

Both verified against source (`main.rs` serve handler 6345-6607, `commands/serve.rs`).

- F1 [medium] `--log` is daemon-only — the top-level `log_file = log.map(|l|
  jit_dir.join(l))` (main.rs:6360) is passed ONLY to `ServeOptions` in the
  daemonize branch (6534-6540). The `--fg` branch runs `cmd.status()` (main.rs
  ~6511) and inherits the terminal's stdio; it never uses `log_file` and prints
  "foreground, Ctrl+C to stop". Flag row now scopes `--log` to daemon mode and
  states `--fg` output stays on the terminal.
- F2 [medium] `error` status covers START failures too — the daemonize branch's
  `Err(e)` arm (main.rs:6592-6604) emits `{"status":"error","error":...}`,
  identical to the stop-failure (6388) and status-failure (6436) arms. Taxonomy
  broadened from "`--stop`/`--status`" to "start, stop, or status".

### Complete status-emission map (every `"status"` the serve handler can emit)

| status        | branch / trigger (main.rs)                                              | payload                                    |
|---------------|------------------------------------------------------------------------|--------------------------------------------|
| `stopped`     | `--stop`, StopOutcome::Stopped (6364)                                   | `pid`, `port`                              |
| `not_running` | `--stop` NotRunning (6378) / `--status` None (6426)                     | —                                          |
| `error`       | `--stop` Err (6388) / `--status` Err (6436) / daemon-start Err (6592)   | `error` (message)                          |
| `running`     | `--status` Some (6410) / fg already-running (6463) / daemon AlreadyRunning (6578) | `pid`, `port`, `url` (+ `log_file`,`started_at` in the `--status` view) |
| `exited`      | `--fg`, after `cmd.status()` returns (6512)                             | `port`, `exit_code`                        |
| `started`     | daemonize, ServeOutcome::Started (6542)                                 | `pid`,`port`,`url`,`log_file`,`web_ui`,`web_ui_source` |

Set is complete — the six documented values match the six the handler emits, no more.

### Mode-conditional behavior (daemon vs `--fg`), re-confirmed

- **Logging:** daemon → `--log` file (default `.jit/server.log`); `--fg` → terminal stdio, `--log` unused. (F1)
- **Blocking / Ctrl+C:** daemon returns immediately after launch; `--fg` blocks inline until Ctrl+C or the server exits (`cmd.status()`).
- **Status emitted on a fresh run:** daemon → `started`; `--fg` → `exited` (only after the run ends). An already-running server short-circuits to `running` in both modes.
- **PID file:** written/tracked by the daemon path (`start_server`); the `--fg` path reads it for the already-running check but the inline run is not a tracked daemon.
- **Unconditional (same in both modes), re-confirmed against source:** port scan (`start..=start+99`, serve.rs:238-258), `--web-dir` resolution + auto-detect (`find_web_dir`), `/api` + `/` endpoints, filesystem-vs-embedded UI selection. Nothing else is stated unconditionally that is actually mode-conditional.
