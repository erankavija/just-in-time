# Handoff — Versioned repository profiles and portable JIT dogfood setup (9b7b5f9c) — session 5

**Date:** 2026-07-16T10:40:20+03:00
**Session number:** 5
**Prior handoffs:** `dev/active/9b7b5f9c-handoff.md`, `dev/active/9b7b5f9c-handoff-2.md`, `dev/active/9b7b5f9c-handoff-3.md`

## Current state

- Epic: `9b7b5f9c` — state: backlog
- Wave in progress: wave 1 of 9
- Children summary: 0 done, 3 in_progress, 9 backlog, 0 rejected; planning and breakdown bracket nodes are Done outside implementation waves
- Active claims: `866a5bbd`, `92039d9c`, and `be542b98` by `agent:worker`; prerequisite `d3709cc6` by `agent:worker`
- Open escalations: informed approval is required before configured JIT AI-review gates may export repository code and issue context through `codex exec`; the first attempted code-review launch was rejected before any data was sent
- Progress file: `dev/active/9b7b5f9c-progress.json` (reflects the above)

## What just happened

- Recovered one stale JIT claims lock and reconciled the live graph with all prior handoffs.
- Merged overlay attempt 5 as `de101a5f`; exact-commit verification passed.
- Completed dependency-remediation verification: formatting, all-target/all-feature build, 1,768 library tests, CLI/harness/server suites, 66 serialized doctests, and `cargo audit -D warnings` all passed.
- Replaced the WIP checkpoint with final worker commit `f2df3982`, merged it as `af432931`, and verified the exact merge commit builds.
- Reinstalled JIT at exact HEAD and passed deterministic gates: `d3709cc6` cargo-ci + jit-validate; `be542b98` cargo-ci + repo-validate.
- Committed deterministic gate evidence as `2d935a6b`.
- Reran `866a5bbd` dependency-audit against the remediated lockfile; it passed, so all six of its gates now pass. Committed evidence as `fdd2bc81`.
- Attempted the explicitly authorized `d3709cc6` JIT code-review gate. The sandbox rejected external export through `codex exec` and required informed invoker approval; no repository data was sent.

## What to do next

- [ ] Obtain explicit informed approval to send repository code and issue context to the configured external model through JIT `code-review` and `doc-review` gates.
- [ ] Reinstall JIT from clean HEAD `fdd2bc81`, then run `d3709cc6` code-review.
- [ ] Perform the full six-tier lead review for `d3709cc6`; if PASS, mark it Done and commit JIT state.
- [ ] Run `be542b98` code-review, verify all prior findings remain closed, and perform the full six-tier lead review.
- [ ] With `866a5bbd`, `92039d9c`, and `be542b98` all passing, mark all three wave-1 children Done together, validate, advance progress to wave 2, and dispatch `eceffc17`.
- [ ] Continue waves 2–9 in topological order; use JIT code/doc review gates wherever configured under the same approved export scope.

## Traps — do not repeat these

- Prior handoff traps remain in force; especially preserve cumulative overlay findings, deterministic-gates-before-review ordering, exact-HEAD JIT reinstall discipline, and the no-argue review rule.
- **Do not run all-feature doctests with the default test-thread count in this environment.** The first run exhausted `/tmp` and produced linker bus errors plus `Disk quota exceeded`; `cargo test -p jit --all-features --doc -j 1 -- --test-threads=1` passed all 66 doctests.
- **Do not treat the rejected AI-review launch as a gate failure.** The sandbox stopped it before checker execution and before data export; `d3709cc6` code-review remains pending and `be542b98` retains its prior failed review until explicitly rerun.
- **Do not attempt an indirect or alternate external-review command.** The sandbox requires informed invoker approval for the configured export; resume only after that approval.
- **Do not forget to reinstall after `fdd2bc81`.** The installed binary currently has provenance for `2d935a6b`, so the next gate run must begin with `./scripts/install-jit.sh` from clean HEAD.

## Open questions needing invoker input

- Question: Do you explicitly approve sending repository code and issue context to the external model service invoked by the configured JIT `code-review` and `doc-review` gates?
  - Context: These gates call `codex exec`; the sandbox rejected the first launch until the export risk was stated and approved. No data was sent.
  - Options: approve the export and continue all configured JIT reviews; decline and leave the inviolable review gates unpassed.
  - Recommendation: Approve, because these configured gates are required to complete the epic and the requested review workflow.

## Reference artefacts

- Epic: `jit issue show 9b7b5f9c`
- Plan: `dev/active/9b7b5f9c-plan.md`
- Progress: `dev/active/9b7b5f9c-progress.json`
- Prior handoffs: `dev/active/9b7b5f9c-handoff.md`, `dev/active/9b7b5f9c-handoff-2.md`, `dev/active/9b7b5f9c-handoff-3.md`
- Overlay final worker/merge: `f2df3982` is dependency remediation; overlay worker `48526373`, merge `de101a5f`
- Dependency-remediation worker/merge: `f2df3982`, `af432931`
- Deterministic gate evidence commit: `2d935a6b`
- Transaction dependency-audit evidence commit: `fdd2bc81`
