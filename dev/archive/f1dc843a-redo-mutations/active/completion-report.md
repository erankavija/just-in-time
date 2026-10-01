# Epic Complete: Mutations commit through a redo record and never scale with project history (f1dc843a)

**Started:** 2026-09-20
**Completed:** 2026-10-02
**Assignee:** agent:jit-execution-lead

## Summary

Every repository mutation now reads its inputs in one capture round per session and publishes through one synced redo record that a session applies at open. The journaled transaction kernel, its backups, staging, reverse actions, worktree companions, plan hash, phased capture, listing fingerprints and mutation document closure are gone. A changing label update dropped from 118–122 ms to 49 ms, and stays at 50 ms on a ten-times larger event log.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 50 / 50 (epic children, bracket nodes and story checkpoints included) |
| Waves executed | 23 planned waves plus the bracket |
| Rework cycles | 23 rounds across 19 issues (from `progress.json` `rework_counts`) |
| Escalations | 21 (ESC-01..ESC-20 plus ESC-09-followup), all resolved by the owner |
| Sessions | 8 |
| Issues created during execution | 10 (9 inside the epic, plus adb5e9df driven alongside at the owner's request) |
| Cumulative Rust balance from `34d37f8be` | +12,053 / −14,279, net −2,226 (redo-record-only subtotal −2,026) |

## Success Criteria

- [x] REQ-01 — label update under 100 ms on both repositories, median of three, committed artifact with changing, idempotent and lease-refused scenarios. Delivered by 722878d4 (`dev/benchmarks/publication-cost-722878d4/`): 49 ms on gf2 and on this repository. The harness ships as `scripts/benchmark-mutation-cost.sh` (ef7848ea).
- [x] REQ-02 — ten-times log differs by under 10 %. 50 ms against 49 ms (+2 %). Delivered by c88822a5 (length-and-tail log capture), b4483ad6 (positioned append) and 722878d4 (evidence).
- [x] REQ-03 — one redo record per mutation, interruption at every step converges on both backends. Delivered by 835bae3f, 555ff4be, 35544121, 4e818d10, 8559f529, d591a1c8 (conformance crash matrix), 4319256e, 10ba2e35, 42478e5d; checkpoint 21e976d1.
- [x] REQ-04 — a changed written file, log length or tail returns a retryable conflict and changes nothing. Delivered by faa668c9 (`ensure_write_set_unchanged`) and wired in 4319256e; the tail bounded by its final line per `@/issue/c88822a5/decision/D-1`.
- [x] REQ-05 — the ten constructs are gone, with no second publication path beside the record and `jit init` as the one exception. Delivered by b0ee2dd6 (kernel −4,367 lines), ec72359b, c88822a5, 33f97287, 8454236c, 51eb1aa6, 3d9150d7 and dd1b984c (lock and identity writes never create the root). Checked construct by construct in `dev/active/e590fa91/checkpoint-evidence.md`.
- [x] REQ-06 — Rust workspace net removal from `34d37f8be`. Net −2,226, recorded in the artifact's balance section (`rust-balance.py`).
- [x] REQ-07 — invariants, storage reference and publication/recovery documents state the redo record, and none describes journaled rollback. Delivered by 816df12a, ccea63de, ead8f86d, 79252d23, e6e25f9a, b4483ad6, e346d387, 91168745, be13da79 and 8b6a9e34; checkpoints e590fa91 and ef7848ea.

## Wave Execution Log

- **Waves 1–6:** record vocabulary, durable write, conflict check, file and memory apply, overlap validation and the shared conformance suite. Checkpoint 21e976d1.
- **Waves 7–12:** publication cutover: failure injection, session commit through the record, the init publisher and kernel retirement. Checkpoint fbd6c859 at wave 15.
- **Waves 13–14:** documentation of the record, the troubleshooting procedure, and `jit recover` reporting `recovery_failed`.
- **Wave 16:** source guards, transaction fields leaving command output, positioned append, single-pass capture (c88822a5, five review rounds).
- **Waves 17–19:** plan hash, listing fingerprints and document closure retired; Rust prose and help; invariant registry and charter; generated storage-records reference; architecture note.
- **Wave 20:** checkpoint e590fa91, which also retired the journal-shaped benchmark script.
- **Waves 21–23:** publication-cost artifact, testing guide, SP-28 remediation (dd1b984c), and checkpoint ef7848ea.

## Key Decisions

- The event log is captured as its length and a tail bounded by its final line (ESC-05, then ESC-15 / `@/issue/c88822a5/decision/D-1`). A whole-log read cost 0.89 ms/MB against a 0.34 ms/MB budget.
- The record gained a make-directory operation (ESC-09), and D-17 was restated at source (816df12a REQ-07).
- REQ-07 on c88822a5 was judged on all lines. Duplicate tests were removed and partial ones moved to their owning layer rather than deleted (ESC-17).
- A whole-repository precheck's stability covers only what the transition reads (`@/issue/8454236c/decision/D-1`, ESC-19).
- The journal-shaped benchmark was retired inside checkpoint e590fa91, because that checkpoint's REQ-04 named it. The plan had placed it in 8b6a9e34.
- SP-28 became task dd1b984c before the epic gate, because epic REQ-05 names its subject.

## Escalations

All twenty-one are recorded in `progress.json` `escalations`. Session 8 raised ESC-14 to ESC-20:

- ESC-14 — the storage reference regained one event-log clause (`@/issue/ccea63de/decision/D-2`).
- ESC-15 — the log tail is bounded by its final line.
- ESC-16 — e346d387 REQ-04 admits names of live mechanisms.
- ESC-17 — the c88822a5 rework counter was reset; tests were deduplicated and relocated.
- ESC-18 — `BUSY_HOST_OVERRIDE=1` was allowed for that session only.
- ESC-19 — broad-precheck stability was narrowed.
- ESC-20 — adb5e9df was driven under v1.0 and gained manifest key substitution.

## Issues Discovered During Execution

- ce053478, 19e7298a, 2d4fda72, 48e4f65c, 7226f00b, b7c99dcb, 434a60b7 — remediation and spike tasks created under POL-01 or for measurement (sessions 1–6).
- 8559f529 — a record rejects an overlapping operation set (session 2).
- dd1b984c — lock and identity writes never create an absent data root (session 8, SP-28).
- adb5e9df — remove the unqualified `satisfies` form. Filed by the gf2 session and parented under milestone v1.0, not this epic. This repository's 678 labels were migrated with `dev/active/adb5e9df/migrate-satisfies.py`.

## Open items for v1.0 triage (surfaced, not named by any criterion)

- SP-03 — `commands/document.rs` keeps a hand-picked event-log capture ceiling.
- SP-04 — six listing-expanding capture budgets keep hand-picked byte ceilings (256–512 MiB).
- SP-06 — a stale `#[allow(unused_imports)]` on the memory re-export.
- SP-07 — a symlink at an operation's target diverges between backends by design (the memory backend has no symlinks).
- SP-18 — the init stage name `<leaf>.stage` is deterministic.
- SP-20 — the worktree bootstrap lock now serializes only absent-root publication.
- SP-22 — the deployment guide's backup sweeps machine-local state.
- SP-30 — profile preparation lost its same-round closure-race test.

## Holistic Quality Notes

- `cargo-ci` failed its first cold run after nearly every merge under external host load. The signature was the same six fixture tests timing out. Warm re-runs passed. The suite-duration budget needed the session-scoped override, and the 274 ms cold fixture-cache margin was accepted by the owner (`dev/benchmarks/rust-build-budgets/README.md`).
- Doc-review and holistic-review repeatedly caught prose stating the redo application order differently from `redo_apply.rs`. The order is make directories, puts, deletes, append, remove record. Three pages now agree with the code and with D-17.
- Moving a harness changes the asset metadata recorded on every issue that links a document citing it. Re-adding the links refreshed 46 issues.
