# Core Maintenance epic (6eb585bc) — Usability Audit & Session Mining (2026-07-05)

Two-part audit: (1) hands-on verification of the 8 fixes delivered in the 2026-07-05
batch, (2) mining of 17 session transcripts (2026-07-01 … 2026-07-05, ~900 jit
invocations) for residual CLI friction. Six parallel agents: one behavioral fix
auditor, one CLI-surface reviewer, four transcript miners.

## Part 1 — Fix verification

All 8 delivered fixes verified **works as claimed** against a freshly built binary
(commit c80c8378) in isolated scratch repos:

| Issue | Verified behavior |
|---|---|
| ff9925af | Root discovery from nested dirs; `.git`-boundary respected; `JIT_DATA_DIR` precedence |
| 8a15088a | Same-assignee re-claim idempotent; promotes to in_progress; foreign claim still rejected |
| b2f9f390 | Append/file/stdin round-trip incl. 5000-char body; batch-mode guard rejects |
| f847df3f | Delete strips dangling edges (validate clean); dep-rm ambiguity leg code-verified |
| 7a50e021 | Redundant edge exit 4 naming the pair; `--reduce` drops shadowed edge; variadic fails nonzero |
| 4af511fd | `JIT_ISSUE_DOCS` delivered to exec gates; `[]` for doc-less issues; documented |
| def64ac4 | schema_version 9999 → exit 10 on every entry path incl. `gate list`/`init` |
| 03d54a9f | Clock threaded through production paths; both deflaked tests pass in ~0.02 s |

Residues found by the audit: variadic `dep add` partial-commits before rejecting
(filed as c8518f2a); `claim_coordinator.rs is_stale` still reads `Utc::now()`
directly (reviewed, **not filed** — invoker decision); stale help/doc text and
`--json` parity gaps (filed as 1a63ef75).

## Part 2 — Friction clusters from mining

1. **Status projection & rollup** — ~125 hand-rolled `show --json | jq/python`
   one-liners for state+gates+unmet-deps; ~30 per-child loops/Counter rollups for
   container progress.
2. **Gate inspection loop** — ~48 freetext scrapes of checker stdout for
   findings/verdict; `key` vs `gate_key` field distrust; existing `gate status-all`
   and history view unused for lack of cross-references.
3. **JSON shape guessing** — `{count, issues:[…]}` vs bare-array confusion caused
   silently empty jq projections and verbatim-repeated defensive parsers; field
   names reverse-engineered via `jq keys`.
4. **Bulk-export bypass** — one external-tooling session never invoked jit: export
   too thin, lifecycle timing only in the event log, hierarchy/clustering only in
   the web UI TypeScript.
5. **CLI surface inconsistency** (live-confirmed) — three removal-verb spellings,
   assignment/lease verb overload, non-repeatable `--label` in the query family,
   prefix errors exiting 1 instead of 2, `Error: Error:` double prefix, `init`/
   `graph export` without `--json`.

## Issues filed (15, all children of 6eb585bc)

Commands: cc42a69b (status projection), 7fe5c743 (child rollup/aggregation),
27338abc (structured gate findings), 0ab468ba (full export behind `--full`),
31e12d2b (hierarchy in core).
Consistency: d0f88ee2 (verb aliases), dc3bef62 (repeatable `--label`),
a05b87ae (typed exit codes + startup JSON envelope), 30a3b5c1 (error-message polish).
Schema/discoverability: 74fbdb69 (uniform list envelope), b1586c0d (gate field
naming), 043ae624 (config accessor), 62f3bebd (help cross-references).
Follow-ups: c8518f2a (atomic variadic dep add), 1a63ef75 (stale help/doc sweep).

Design decision (invoker): bulk-emitting commands keep lean default output; complete
records are a uniform `--full` opt-in. Single-issue commands stay full-fidelity.

Raw extraction data and miner scripts were session-scratch (not retained); evidence
frequencies above are recorded in the filed issue descriptions.
