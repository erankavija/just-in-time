# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 8

**Date:** 2026-08-10T12:23:48+03:00
**Session number:** 8
**Prior handoffs:** `dev/active/c639cfb5-jit-profiles-complete/handoff.md` through `dev/active/c639cfb5-jit-profiles-complete/handoff-7.md`

## Current state

- Epic: `c639cfb5` — state: backlog
- Current wave: wave 9 of 18; Wave 8 is complete
- Children summary: 10 done, 0 in progress, 14 backlog/ready, 0 rejected
- Next issue: `7156f64b` — **Ready** — “Expose reconfiguration and upgrade as commands”
- Active claims: none
- Open escalations: none
- Main implementation integration: `47083065` (`merge(jit:44ec7192): decide safe profile replacements`)
- Progress ledger: `dev/active/c639cfb5-jit-profiles-complete/progress.json`

## What just happened

- Completed Wave 8's two independent issues in isolated worktrees, using Terra xhigh for the cross-cutting three-way decision and Luna xhigh for the bounded lifecycle-event implementation. No Sol agent was used.
- `9f493686` replaced live per-package `ProfileApplied` construction with one aggregate `ProfileLifecycle` event. The event names the requested operation, carries stable per-profile installed/unchanged/reconfigured/upgraded outcomes, reports variable names and source kinds without values, and includes encountered shipped-record conversions. Historical `ProfileApplied` records still decode.
- The event issue used two bounded rework rounds. First, full cargo-ci exposed four stale integration assertions that still expected one event per package or erased the `initialize`/`apply` distinction; commit `450cead5` corrected those contracts. Second, code review required direct coverage for upgraded, reconfigured, and unchanged profiles in a changed aggregate; commit `607175f9` added that classifier regression. Final source/evidence head `765bafce` passed cargo-ci run `c988f514-a975-44eb-aa43-6d885e8522b7` and code-review run `c2f4b2a4-7d8f-49b9-91d8-7eaba3c4b411`, then merged as `9a10b933`.
- `44ec7192` added one pure exhaustive base/current/candidate decision table and integrated it into aggregate profile planning. Current-equals-base updates safely; divergence produces a typed actionable conflict and publishes nothing; stopped shared/adopted/changed claims retain content; only an unchanged, solely-owned, unretained stopped asset becomes `DeleteFile`; changed packages rewrite their applied records while unowned content survives.
- The three-way implementation reuses the canonical profile-asset fingerprint rather than defining a second hash recipe. Its stopped-claim decisions flow through the existing aggregate action fold and recoverable transaction; no transaction framework or Wave 9 command surface was added.
- Merged lifecycle events into the three-way branch before final validation. Exact combined commit `1a6f8416` passed cargo-ci run `6b7af235-68b1-444d-b224-1d3095dedf5a` and code-review run `7c5630d1-6af1-4a09-8ad2-24132e4f5d52` with zero findings, then `44ec7192` moved to Done and merged as `47083065`.
- One earlier three-way cargo-ci record, `561e0edb-7efa-4687-8435-eda4a597b987`, is preserved but is not source evidence: the lead reused the lifecycle worktree's Cargo target, so Cargo retained newer debug artifacts and repeated the lifecycle branch's four failures byte-for-byte. The authoritative rerun used a fresh isolated target and passed.
- Temporary-build quota was recovered by deleting only completed Cargo caches. Two orphaned `sccache` daemons with PPID 1 retained the shared lock after completed gates; each was verified with `lslocks`/`ps` and stopped with `sccache --stop-server` without touching a live build.

## What to do next

- [ ] Reinstall an exact clean main binary with `./scripts/install-jit.sh` before any JIT mutation. `/tmp/jit-wave8-three-way-install/bin/jit` embeds combined worker commit `1a6f8416`, not current main, so the stale-binary guard must reject it for claims or gates on main.
- [ ] Claim only `7156f64b`, commit its claim/progress transition on main, then create one explicit isolated worktree from that exact commit. Do not dispatch Wave 10.
- [ ] Use Terra xhigh for `7156f64b`: it spans Rust CLI/domain/storage behavior plus the MCP contract and has required gates `cargo-ci`, `mcp-ci`, and `code-review`. A bounded Luna read-only seam audit is reasonable; Sol is unnecessary unless a genuinely new architecture problem appears.
- [ ] Put `profile reconfigure` and `profile upgrade` directly over the existing aggregate planner, three-way decision, and lifecycle operation enum. Do not add a second mutation path, decision engine, transaction wrapper, record format, or event schema.
- [ ] Reconfigure from the installed record's package identity with changed supplied public inputs; update only affected owned targets. Upgrade to a newer package version must validate compatibility/range constraints against every surviving applied profile before publication.
- [ ] Both commands need rehearsal output over the same planned decisions and must write nothing. A divergent target must remain a typed non-zero conflict with no partial publication.
- [ ] Update the generated CLI/MCP schema through the repository's canonical generation path, then run the exact issue gates. Keep output collections in the required JSON envelopes and preserve Git-optional behavior.
- [ ] After Wave 9 gates and holistic review, update this ledger and continue the planned graph. The invoker explicitly requested this handoff instead of immediate Wave 9 dispatch.

## Traps — do not repeat these

- **Never share one Cargo target across worktrees.** Cargo can retain a newer worktree's debug artifacts when switching absolute source roots, yielding a plausible but invalid gate result. One active worktree gets one target; delete a completed target before allocating the next.
- **The host-wide Cargo lock can survive its parent through `sccache`.** Use `CARGO_INCREMENTAL=0` and `CARGO_CI_NO_SCCACHE=1`, but still inspect `lslocks` and `ps` when a command is silently queued. Stop only a verified PPID-1 cache daemon after confirming no Cargo/rustc parent is alive.
- **Do not treat `561e0edb-...` as a three-way failure.** It is retained for auditability but used contaminated target artifacts. The isolated combined run `6b7af235-...` is authoritative.
- **Do not resurrect `ProfileApplied`.** It remains only as a historical decoder/catalog boundary and explicit historical fixtures. New changed mutations emit one `ProfileLifecycle`; complete no-ops and rehearsals emit none.
- **Repair-only effects do not imply reconfiguration.** Per-profile status comes from installed identity/version and resolved input changes. A coupled repair can make the aggregate changed while that profile's lifecycle status remains `unchanged`.
- **Do not make three-way comparison stringly or hash twice.** Use the typed owner/target inputs and canonical claim fingerprint already shared by record construction and comparison.
- **Removal is intentionally narrow.** Only exact former asset claims are deleted when unchanged, sole, and unretained. Managed regions remain composed; deleting their containing file would be unsafe.
- **Do not pull final adopter documentation forward.** `docs/reference/profiles.md` still describes the old apply-only/event surface; Wave 15 issue `b3d92595` explicitly owns the canonical lifecycle rewrite after the command/presentation surface stabilizes. Current event-schema documentation is already generated and accurate.
- **Respect the two-round rework budget.** `9f493686` used both rounds and is green; `44ec7192` required no source rework. The contaminated-target rerun was infrastructure correction, not issue rework.
- **Keep patches worktree-explicit.** Session 7 proved that relying on a shell workdir for patch routing can leak edits into main. Name the full worker-worktree path in every patch and inspect both statuses.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Next issue: `jit issue show 7156f64b`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`, `breakdown.json`, and `progress.json`
- Prior handoff: `dev/active/c639cfb5-jit-profiles-complete/handoff-7.md`
- Lifecycle source commits: `28e8ca64`, `450cead5`, `607175f9`; final evidence head `765bafce`; main merge `9a10b933`
- Three-way source commit: `70d474ec`; exact integrated gate head `1a6f8416`; main merge `47083065`
- Exact authoritative combined gate runs: cargo-ci `6b7af235-68b1-444d-b224-1d3095dedf5a`, code-review `7c5630d1-6af1-4a09-8ad2-24132e4f5d52`
- Completed worktrees: `.agents/worktrees/agent-9f493686`, `.agents/worktrees/agent-44ec7192`
- Remaining temporary target: `/tmp/jit-wave8-three-way-target` (reproducible cache; safe to delete before the next worktree)
