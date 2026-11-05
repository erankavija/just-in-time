# Handoff — Run the server and web UI on the core's shared mechanisms (0f8efc37) — session 1

**Date:** 2026-11-05
**Session number:** 1
**Prior handoffs:** None. Planning handoff: `execution-handoff.md`.

## Current state

- Epic: `0f8efc37` — state: backlog; direct dependencies: one done, two backlog.
- Wave in progress: ready-DAG interleaving across waves 2–5, explicitly authorized by the steward; all currently actionable implementation leaves are done.
- Outstanding frontend leaves: `d4172016` backlog, blocked by external `933ff852`; `38df8a6c` in progress, awaiting explicit human approval.
- Active claim: `38df8a6c` by the execution lead, since 2026-11-04; its clean branch and worktree remain isolated.
- Open escalations: installer allowScripts authorization and external Git document-diff prerequisite, both owned by the steward.
- Progress file: `progress.json` in this directory.

## What just happened

- Delivered the foundation, shared server mechanisms, semantic search, browser refresh/search/layout/summary work, and bounded execution-discovered browser repairs; merged gates passed and reviewed branches landed on main.
- Published @/charter/D-25 and converged shared reads on bounded retryable conflict. Issue `6326532e` passed full Cargo and review, merged as `abaa6bdf8`, and passed merged Cargo `22112c9d`; the locked installed CLI and its writer-lock/pending-redo scratch probe are documented in `nonblocking-install-verification.md`.
- Browser `b067c2db` and `83b561ed` passed branch and merged web gates; `83b561ed` merged as `08e86ebbb`, merged web run `ae213382`.
- Canonically reclaimed completed frontend worktrees and their owned temp directories; branches remain for history. The blocked `38df8a6c` worktree remains.

## What to do next

- [ ] Wait for the steward to confirm `933ff852` is done, then claim and dispatch `d4172016` from fresh main. Inspect its issue and the authoritative plan; run configured Cargo and review gates and merged checks before completion.
- [ ] Wait for trusted user approval of the exact `web/package.json` `allowScripts` entry `esbuild@0.25.12: true`. Only then resume `38df8a6c` in its isolated worktree, follow configured gates and source-slot coordination, and test a clean install without suppression.
- [ ] Once dependencies finish, verify `c8e79465` and `3a61ff16` criteria against all children and run their configured gates. Reconcile the `abf68973` advisory recorded in `progress.json` against server-story REQ-01; the `satisfies` namespace denotes contribution, not exclusive fulfillment.
- [ ] Run epic holistic acceptance and completion workflow only after all required story dependencies are done. Follow direct-main source slots, serialized merged checks, and steward guidance.

## Traps — do not repeat these

- **Do not retry or indirectly reproduce rejected approval actions.** Automatic review rejected the independent `06177584` merge (blocked source commits `d0c98247f`, `7701663df`) and the precise `38df8a6c` installer-policy entry; the latter requires trusted user authorization for that exact script permission. No workaround or blanket allowlist is authorized. Verify branch ancestry and package policy before every merge.
- **Do not infer shipped defaults from this repository's `.jit/`.** Dogfood configuration is repository-local; the production templates are authoritative.
- **Do not hold shared Rust build locks across review or run simultaneous same-issue gate mutations.** Prior review produced `inputs_moved`; freeze source, record test gate evidence, then review. Use disk/NVMe targets, unique short `/dev/shm` test TMPDIR, `/tmp` resource checks, and the steward's `flock` turnstile/full-CI order.
- **Do not silently drop tracker side effects or other agents' append-only events.** A done transition promoted dependent issue files; all were required in the commit. Coordinate tracker boundaries and preserve concurrent events. A three-way merge of independently published D-25 duplicated the same event ID; identical copies were deduplicated, not distinct events.
- **Do not assume D-22's original fallback still governs reads.** Published D-25 supersedes only that fallback; explicit and mutation recovery remain D-17. Failed review `713a548e` is historical evidence, not permission to restore the fallback. The D-25 independent audit also found memory-getter waits and swallowed preview contention, repaired and tested.
- **Do not use a 404 alone to prove a route is retained.** `4538b071` review `7f87a82e` caught that ambiguity; a route-distinguishing request now proves document diff presence.
- **Do not assume a git dependency reduction preserves a direct edge.** Adding `4858de06` with `--reduce` removed the direct `bb03df0a → 6d93ab14` edge; actual reachability was verified through the epic and browser story.
- **Do not treat the automatic reviewer approval rejection as user approval.** The steward requested the trusted user's decision; wait for an explicit response and avoid indirect retries while pending.

## Open questions needing invoker input

- Question: Has the user explicitly authorized the exact `esbuild@0.25.12` `allowScripts` entry?
  - Context: automatic approval review rejected the exact persistent permission; no patch was applied.
  - Options: authorize the specified entry, or decline and choose a different authorized product direction.
  - Recommendation: await the steward's trusted-user signal; do not bypass the rejection.
- Question: When will external `933ff852` be complete?
  - Context: `d4172016` depends on it; its remaining chain includes `1acfa592` and `a273667c`.
  - Options: wait for the Git lead's integration and gates, then dispatch; escalate any genuine blocker to the steward.
  - Recommendation: wait for the explicit dependency release.

## Reference artefacts

- Epic: `jit issue show 0f8efc37`; hierarchy and gates: `jit graph deps 0f8efc37 --depth 0 --json`.
- `plan.md`, `breakdown.json`, `instantiation.json`, `execution-handoff.md`, and `progress.json` in this directory.
- `nonblocking-install-verification.md` in this directory, linked to `6326532e`.
- Steward thread `01a1a886-d199-7093-b096-7acbab6adb48`; Git lead thread `01a1a88a-3bda-7302-8fdf-0662e2095cc1`.
