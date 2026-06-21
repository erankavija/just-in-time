# jit Improvement Findings from Session Mining

> **Provenance**: Mined 2026-06-21 from 42 Claude Code session transcripts across two
> repos — this repo (jit self-use) and `../gf2`, jit's primary downstream consumer.
> A multi-agent workflow extracted an ordered jit-command-and-outcome digest per
> session, fanned out one analyzer per session, and synthesized the clusters below.
> 209 raw findings from 37 of 42 sessions. Counts are distinct-session frequencies.

## Executive Summary

The single biggest recurring failure pattern is **`jit gate pass` not reflecting the gate verdict in its exit code**: in at least 21 sessions agents could not branch on `$?` after `gate pass`, were silently misled about whether a gate passed, and were forced to follow every `gate pass` call with a separate `jit gate check` — doubling the command count for every gate invocation. The single biggest unintuitive-sequencing problem is that **`gate pass` returns immediately when the checker is async, with no built-in wait primitive**, forcing agents to write bespoke `sleep N; poll` loops with guessed durations (45s to 220s), often overshooting or undershooting, and then still requiring a second command to read the verdict. Together these two issues account for the majority of wasted commands, retry loops, and agent confusion across both repos.

---

## Top Failure Patterns

### 1. `jit gate pass` exit code does not reflect the gate verdict

**Seen in**: 21 sessions (7 jit-self / 14 gf2). The most frequently occurring single bug in the dataset, with clusters of 4–20 occurrences per session.

**Evidence**:

```
> jit gate pass 9138d86c code-review 2>&1 | tail -3
    -> ok
> jit gate check 9138d86c code-review 2>&1 | tail -80
    -> [!] Gate 'code-review' last run: failed (exit code: 1) Duration: 288992ms
```

```
> jit gate pass bc86f54c code-review 2>&1 | tail -1
    -> [!] Error: Gate 'code-review' failed for issue bc86f54c-6372-44fe-b375-3e3a882a73e0.
       Checker status: Failed, exit code: Some(1).
       Inspect details with: jit gate check bc86f54c-6372-44fe-b375-3e3a882a73e0 code-review
bc86f54c: {'cargo-ci': 'passed', 'code-review': 'failed'}
```

Two sub-variants make this worse: in some sessions `gate pass` exits 0 even when the checker recorded FAIL (silent failure); in others it exits 4 when FAIL'd but `gate check-all` then shows a stale FAIL even after a `gate pass` that actually recorded PASS — the display is not consistent with the recorded state (gf2-f339467e).

**Fix**: Define and document three exit codes for `jit gate pass`: `0` = verdict recorded as `passed`; `1` = checker ran and recorded `failed`; `2` = infrastructure/runner error (no verdict recorded). Never exit 0 when the stored verdict is FAIL. Add a top-level `verdict` field (`"pass"` | `"fail"` | `"error"`) to `--json` output. Fix the human-readable error to use the short ID, not the full UUID.

### 2. `jit gate pass` on failure does not show checker output inline — mandatory `gate check` follow-up

**Seen in**: 18 sessions (6 jit-self / 12 gf2). Appears as a tight two-command pair in every session.

**Evidence**:

```
> jit gate pass 42eac5cc code-review
    -> [ERR] Exit code 4 Error: Gate 'code-review' failed for issue 42eac5cc-...
       Inspect details with: jit gate check 42eac5cc-d805-410e-9b1b-a269d51b2f0f code-review
> jit gate check 42eac5cc code-review 2>&1 | tail -60
    -> [!] Gate 'code-review' last run: failed (exit code: 1) ... stdout: **Findings** ...
```

In gf2-d1bb58ee this round-trip happened at least 10 consecutive times. In jit-ef0c5448 the same code-review gate failed 5 times and each failure required a separate `jit gate check --full`.

**Fix**: When `jit gate pass` ends with a FAIL verdict, print the first ~40 lines of checker stdout/stderr inline before the error message (bounded). Add `--quiet` to suppress for scripted use.

### 3. `jit --json` output schema is unstable — `short_id` missing, envelope keys differ per command

**Seen in**: 18 sessions (8 jit-self / 10 gf2). Clusters of 1–21 occurrences each; causes Python `KeyError` crashes.

**Evidence**:

```
jit issue show $id --json | python3 -c "print(d['short_id'])"   -> KeyError: 'short_id'
# Agent guards every read:
i['short_id'] if 'short_id' in i else i['id'][:8]
issues = d.get('issues', d) if isinstance(d, dict) else d
r = d.get('result', d); out = r.get('stdout', '')
```

Confirmed inconsistencies:
- `short_id` absent from freshly-created and `done`-state issues — jit-c489cb5a, jit-cf358320, gf2-35a9d8bf, gf2-0094a880
- `jit issue show --json` root sometimes bare object, sometimes `{"issue": {...}}` — gf2-8e20f3f2
- `jit query all` uses `issues`; `jit search` uses `results`; sometimes bare array — gf2-6cec05ca, jit-7f43b0df
- `jit gate check --json` stdout sometimes top-level, sometimes under `result.stdout` — jit-d2a39298 (21 occurrences)
- `jit gate check-all --json` top key varies: `gates` / `results` / bare array — gf2-e1c163e7, jit-87adbab1
- `labels` absent vs `[]` when empty — gf2-fdc60118, jit-ef0c5448

**Fix**: Enforce a stable JSON contract: (a) `short_id` always present, derived from `id[0:8]` at serialization; (b) `jit issue show --json` returns the issue at root; (c) all list commands use `{"issues": [...]}`; (d) `jit gate check --json` always `{"verdict","status","stdout","stderr","exit_code"}` at root; (e) `labels`/`dependencies`/`gates_required` always `[]` when empty. Add contract tests per lifecycle state.

### 4. `jit gate check` / `check-all` bury the verdict in prose; no structured `verdict` field

**Seen in**: 16 sessions (5 jit-self / 11 gf2). Agents used `grep`, `tail -N`, `sed -n '1,Np'` to extract verdicts.

**Evidence**:

```
> jit gate check-all 026fc832 2>&1 | grep -iE "gate '|passed|failed"
    -> Gate 'cargo-ci' last run: failed (exit code: 1) I'm waiting on the delegated
       diff-scope analysis ...   # grep matched mid-prose, not the verdict line
> jit gate check 5f12e7ff code-review 2>&1 | sed -n '1,55p'   # then 1,50p / 1,40p / 1,45p — all guessed
```

**Fix**: `jit gate check --json` always emits `{"verdict","status","exit_code","stdout","stderr","duration_ms","run_id"}` at root. Human output prints `[PASSED] code-review` / `[FAILED] cargo-ci` as the first line, before checker prose. Add `--tail N` to bound output. Add `verdict` to each `gate check-all --json` entry.

### 5. `jit validate` fires isolated-issue errors on terminal-state issues and cannot be scoped

**Seen in**: 10 sessions (all gf2).

**Evidence**:

```
> jit validate 2>&1 | tail -5
    -> [!] Error: Found 1 isolated issue(s) not connected to the dependency graph:
       'a7b1bb21' (Implement DVB-T2 §6.1.4 bit-to-cell-word demuxer)
```

Agent annotated their script `(expect only the DVB-T2 a7b1bb21 isolated warning)` — could not silence it. The issue was `state = rejected`, `resolution:obsolete`.

**Fix**: Exclude `rejected`/`archived`/`done` issues from the isolated-issue check. Add `jit validate --scope <id>...` to validate only named issues + direct deps. Add `--errors-only` to suppress warnings.

### 6. `jit gate list` / `jit gate status` do not accept an issue ID

**Seen in**: 9 sessions (4 jit-self / 5 gf2).

**Evidence**:

```
> jit gate list 7bdaf999 2>&1
    -> error: unexpected argument '7bdaf999' found  (then fell back to issue show --json + python)
```

**Fix**: Add `jit gate status <ISSUE_ID> [--json]` printing `gate_name | status | last_run_commit` for all gates on the issue; `--json` emits `{"gates": {...}, "all_passed": bool}`.

### 7. `jit gate pass` no-ops silently when already passed; no `pass-all`

**Seen in**: 8 sessions (4 jit-self / 4 gf2). Up to 15 redundant cargo-ci re-runs in one session (jit-d2a39298).

**Evidence**:

```
> jit gate pass 695350fd code-review 2>&1 | tail -10   -> ok   [no code change]
> jit gate pass 695350fd code-review 2>&1 | tail -10   -> ok   # re-ran? no-op? no signal
```

**Fix**: (a) On a gate already `passed` for the current HEAD, emit `already passed at <commit> — skipping`, exit 0 with `already_passed: true` in `--json`. (b) Add `jit gate pass-all <ISSUE_ID>` running all required gates, non-zero if any fail.

---

## Top Unintuitive Tool Sequencing

### 1. No `jit gate wait` — agents poll with bespoke `sleep N; show --json | python3` loops

**Seen in**: 15 sessions (5 jit-self / 10 gf2). Dominant pattern in any async-gate session.

**Evidence**:

```
jit gate pass e4849f07 code-review --by agent:project-lead
sleep 120; jit issue show e4849f07 --json | python3 -c "...['gates_status']['code-review']"
# or a 60-iteration loop:
for i in $(seq 1 60); do s=$(jit gate check 25ad2a02 cargo-ci --json | python3 -c "...print(d.get('status'))");
  [ "$s" = passed ] || [ "$s" = failed ] && break; sleep 10; done
```

Sleep durations observed: 45, 75, 110, 120, 130, 140, 150, 160, 200, 220s — all guessed.

**Fix**: Add `jit gate wait <ISSUE_ID> <GATE_KEY> [--timeout 300]` that blocks until terminal status, exits 0 on pass / 1 on fail / 2 on timeout, prints final verdict + duration. Eliminates every polling loop in the dataset.

### 2. `jit issue claim` blocks on incomplete deps; error omits `jit issue assign`

**Seen in**: 5 sessions (3 jit-self / 2 gf2).

**Evidence**:

```
> jit issue claim f6a704d0 agent:project-lead
    -> Error: Cannot transition to 'in_progress': issue blocked by 4 incomplete dependencies
       To fix: - jit graph deps ...   # never mentions assign; agent fell back to jit issue assign
```

**Fix**: On claim-blocked-by-deps, add to the error: `To assign without transitioning state, use: jit issue assign <id> <assignee>`. Add `--assign-only` to `claim`.

### 3. `jit claim release` requires the opaque lease UUID, not the issue ID; no `--agent-id`

**Seen in**: 3 sessions (all jit-self).

**Evidence**:

```
> jit claim release 8646a474 --agent-id agent:claude-opus   -> error: unexpected argument '--agent-id'
> jit claim release --help                                  -> Usage: ... <LEASE_ID>
> JIT_AGENT_ID=agent:claude-opus jit claim release 6503fb12-a82e-...   -> ok   # 3 tries to succeed
```

**Fix**: Allow `jit claim release <ISSUE_ID>` (look up the active lease for the issue by current agent identity). Add `--agent-id` to all `jit claim` subcommands for consistency with `acquire`.

### 4. `jit issue delete` leaves dangling edges; `validate --fix` won't repair them

**Seen in**: 2 sessions (both jit-self).

**Evidence**:

```
> jit validate --fix 2>&1 | grep -iE "fixed|error|violation"
    -> Error: Invalid dependency: issue '2fbd2a82' depends on '5f2170cc' which does not exist
```

After deleting 12 issues, dangling edges remained; agent wrote a Python loop to repair `.jit/issues/*.json`.

**Fix**: `jit issue delete` atomically removes all dependency edges pointing to the deleted ID. Extend `jit validate --fix` to auto-repair dangling-dependency violations.

### 5. No batch issue creation with DAG wiring — agents write hundred-line Python subprocess loops

**Seen in**: 3 sessions (all jit-self).

**Evidence**: 200+ line Python scripts calling `jit issue create` and `jit dep add` in loops because there is no batch-creation path.

**Fix**: Add `jit issue batch-create --from-json <file>` accepting an array of issue defs (title, description, labels, gates, priority, `depends_on` referencing symbolic keys within the file). Return an array of created IDs.

---

## Other Ergonomics & Output Issues

- **Pervasive python3 reshaping of `--json` (25+ sessions, both repos)** — no `--field` flag and no stable schema. 40 occurrences in jit-7f43b0df alone. **Fix**: `jit issue show <ID> --field <name>` (plain text), `--fields state,title,assignee` (compact JSON), multi-ID `jit issue show <ID1> <ID2> --json` (array), `jit graph deps <ID> --flat --json` (flat array).
- **`gates_required` / `gates_status` split in `issue show --json` (12 sessions)** — two fields to cross-reference. **Fix**: single `gates` array `[{key, status, last_run_at}]`, `status: "pending"` populated on `gate add`.
- **`jit gate check` requires filesystem spelunking (10 sessions)** — agents scraped a UUID path and opened `.jit/gate-runs/<uuid>/result.json`. **Fix**: embed `stdout`/`stderr`/`exit_code`/`duration_ms`/`run_id` inline in `--json`.
- **`jit gate add` noisy on idempotent calls (6 sessions)** — `ℹ Already required` forces `2>/dev/null` everywhere. **Fix**: emit nothing, exit 0 when already registered.
- **`jit recover` triggers false-positive monitors (6 sessions)** — `Warning: Removed stale lock` on stdout mixes with JSON. **Fix**: diagnostics to stderr, drop `Warning:` prefix, add `--json`.
- **`jit issue search` missing `--label`; requires QUERY even with filters (4 sessions)**. **Fix**: add `--label`; make QUERY optional when a filter flag is present.
- **`jit issue delete` requires undiscoverable `JIT_ALLOW_DELETION=1` (3 sessions)** — not in `--help` or the error. **Fix**: add `--yes`/`-y` or document the env var in `--help` and the error.
- **Alias/naming gaps (8 sessions combined)** — `dependency`/`document` as aliases for `dep`/`doc`; `--add-label` alias for `--label`; `jit issue list` redirect to `jit query`; `jit dep rm` to accept full UUIDs; `jit gate status` as a distinct subcommand.
- **`--gate` on `jit issue create` reportedly no-op'd silently (1 session, high severity)** — flag listed in `--help`, accepted, gates never wired; agent discovered `jit gate add` only after inspection. *(Re-verify against current 0.2.1: the flag now carries a full help description.)*
- **`jit gate runs` subcommand missing (3 sessions)** — agents tried `jit gate runs <issue> --gate <key>`, fell back to scanning `.jit/gate-runs/`. **Fix**: add `jit gate runs <ID> [--gate <KEY>] [--json]` returning run history newest-first.
- **`jit graph deps --transitive` / `--flat` missing (4 sessions)** — agents tried `--transitive`/`--recursive` (rejected), then wrote recursive jq/Python. **Fix**: add `--transitive` (BFS closure) and `--flat` (flat JSON array).
- **cargo-ci floods reviewer context with raw test output (1 session, raised by human)**. **Fix**: test-runner gate templates summarize on success, show only failing tests on failure; add a `pass_context_lines` cap to gate definitions.

---

## Prioritized Recommendations

1. **Fix `jit gate pass` exit code** — 0 PASS / 1 FAIL / 2 runner error; document in `--help`. Affects 21 sessions. Highest ROI.
2. **Print checker stdout inline on `gate pass` failure** — first ~40 lines; `--quiet` to suppress. Removes the pass→check round-trip (18 sessions).
3. **Add `jit gate wait <ISSUE> <GATE> [--timeout N]`** — blocks until terminal, exits 0/1/2. Eliminates bespoke poll loops (15 sessions).
4. **Stabilize the `--json` contract** — `short_id` always present; consistent list envelopes; `gate check` `{verdict,stdout,stderr,exit_code}` at root; `labels`/`dependencies` always `[]`. Contract tests per state. 18 sessions.
5. **Add `jit issue show --field <name>` / `--fields <f1,f2,...>`** — addresses the dominant friction in 25+ sessions.
6. **Add `jit gate status <ISSUE_ID> [--json]`** — gate name → status → last_run_commit, `all_passed: bool`. 9 sessions.
7. **Consolidate `gates_required` / `gates_status` into one `gates` array** — `status: "pending"` on `gate add`. 12 sessions.
8. **Exclude terminal-state issues from isolated-issue validation; add `--scope <id>`** — 10 sessions.
9. **Make `jit gate add` silent on idempotent calls** — 6 sessions.
10. **Add `jit gate runs` + embed gate-check stdout/stderr inline in `--json`** — removes `.jit/gate-runs/` spelunking. 6+ sessions.
11. **Fix alias/naming gaps** — `dependency`/`document`, `--add-label`, `jit issue list` redirect, `dep rm` full-UUID, `JIT_ALLOW_DELETION` discoverability. 8 sessions.
12. **Verify/fix `jit issue create --gate` actually wires gates** (or remove the flag); add a `validate` warning for `type:task`/`type:story` issues with no gates. Prevents silent gate omission.
