# Core Maintenance epic (6eb585bc) — Batch Report (2026-07-05)

Umbrella epic for standalone jit core maintenance and bug-fix work. **The epic
stays open as a living container** — new maintenance items will be added under it
over time. This report covers the batch of children driven to completion by the
execution-lead workflow on 2026-07-05; it is not an epic-closure report.

> **Superseded for v1.0 (2026-07-15):** Charter decision `@/charter/D-14`
> replaces the living-container lifecycle instruction above. Finish the current
> core-maintenance scope and close epic `6eb585bc` before the production-readiness
> source-freeze boundary. The original paragraph remains as historical context
> for this batch report, not as current execution guidance.

## Status against the epic criterion

> Every child maintenance item is resolved (done or rejected) with its own verifiable criteria.

**Currently satisfied for all present children.** All 14 children are `done`, each
carrying an explicit `## Success Criteria` section with verifiable `[hard]` REQ
items. Whole-repo `jit validate` passes. The criterion is a standing bar the
container re-meets as new children are added and resolved, not a one-time gate.

## Metrics

- **Children:** 14 total — 6 already resolved before this session, 8 delivered this session.
- **Execution shape:** 8 serial sub-waves (one issue at a time). The 8 open
  children all touched the monolithic `main.rs`/`cli.rs`; parallel worktrees
  would have collided at merge, so work was serialized for reliability.
- **Worker dispatches:** 8 (+1 rework resume).
- **Gate rounds:** every issue passed `cargo-ci` + `code-review`; several needed
  review-driven rework (see below).
- **Escalations:** 1 (invoker-approved criterion amendment on 8a15088a).

## Children delivered this session

| Issue | Title | Rework rounds | Notes |
|---|---|---|---|
| ff9925af | Repo-root discovery walks up like git | 2 | Filed this session. Lead fixed an ancestor-`.jit` binding bug (bind only to an initialized `.jit` with `index.json`); + `# Examples`. |
| 8a15088a | Idempotent same-assignee claim | 0 (1 escalation) | Criterion conflated `jit issue claim` (assignment) with `jit claim acquire` (lease); invoker approved amendment A-8a15088a. |
| b2f9f390 | Append-to-description + file/stdin | 2 | Batch-mode (`--filter`) rejection guard; `# Examples`. |
| f847df3f | Delete strips dangling dependency edges | 1 | `dep rm` ambiguous-prefix rejection. |
| 7a50e021 | dep add rejects redundant edges (`--reduce`) | 1 | Variadic add now fails nonzero on any per-edge error. |
| 4af511fd | Expose linked docs to gate checkers (`JIT_ISSUE_DOCS`) | 0 | First-pass green. |
| def64ac4 | Fail fast on newer-than-supported repo format | 2 | Closed `gate list` and `init` startup-guard bypasses. |
| 03d54a9f | Deflake lease/heartbeat via injectable clock | 1 | Rework threaded the clock through the real production paths (not just test helpers). |

Already resolved before this session: df0b551f, b6aeb05b, 82b8a519, 0cce1a44,
949cd9d0. eff11282 was verified-and-closed this session (already delivered by the
existing `jit issue claim --assign-only`).

## Key autonomous decisions

- **Serial execution over parallel worktrees.** 6 of 8 open children edit
  `main.rs`/`cli.rs`; parallel merges would conflict. Chose one-at-a-time in the
  main tree — reliability over wall-clock.
- **Lead-applied surgical fixes for well-diagnosed review findings** (ff9925af
  ancestor-`.jit`, b2f9f390 batch guard, f847df3f ambiguity, 7a50e021 exit code,
  def64ac4 startup bypasses) rather than a full worker round-trip. The 03d54a9f
  production-path rework was returned to the worker (broader design change).
- **Preemptive `# Examples` instruction** added to later dispatches after two
  issues tripped the doc-contract finding; subsequent issues avoided it.
- **Self-verify with `./scripts/cargo-ci.sh`, not `cargo test`.** ff9925af's
  first submission passed `cargo test --workspace` but cargo-ci caught a real
  regression (the persistent shared `TMPDIR` poison). All later workers were told
  to self-verify with the actual gate script.

## Escalation log

- **8a15088a REQ-01 (criterion amendment).** The criterion required `jit issue
  claim` to "record a lease" and assert "a live lease", but that command is
  assignment-only — leases belong to `jit claim acquire` (git-backed). The
  code-review gate enforced the wording literally. Escalated; invoker approved
  rewording to match reality (assignment persistence + in_progress promotion).
  Amendment recorded in the issue description.

## Issues discovered during execution

- **Persistent CI-temp poison.** `scripts/cargo-ci.sh` sets `TMPDIR` to a
  persistent `~/.cache/jit-cargo-ci-tmp`; a stray bare `.jit` left there by an old
  test deterministically broke ff9925af's discovery until the fix hardened
  discovery to skip un-initialized `.jit` dirs. Logged as a test-hygiene follow-up
  in the progress file (tests writing `.jit`-shaped state into the shared temp
  root).

## Holistic quality notes

Cross-cutting conventions held across the 8 changes: typed `thiserror` errors
mapped to the exit-code taxonomy (redundant-edge and format-mismatch both reuse
established codes), pure/injectable cores (discovery predicate, `DescriptionUpdate`,
`build_issue_docs_env`, the `Clock`/`TickSource` seams) kept logic testable without
I/O, and every new public API carries a `# Examples` doctest. User-facing docs were
swept and corrected per change (content-standards, dependency-management,
core-model, custom-gates, storage-format, cli-commands).
