# Epic Complete: Converge previews and coupled derivations on the mutation session (b697a3f7)

Previews, scoped reads and checkers now run the capture and derivation of the operation they describe. Lock-free reads are admitted only past an unchanged redo record and event log, and return a retryable conflict under contention (`@/charter/D-25`). Every mutation derives its coupled state in one session and publishes one redo record that appends an event, and the duplicated loaders, primitives and rules converge on one form. The epic's gates pass: `repo-validate`, and `holistic-review` on its second run (2026-11-09).

## Success Criteria

Each criterion is delivered by the tasks labelled `satisfies:b697a3f7/REQ-NN`, all done. List them with `jit issue list --label satisfies:b697a3f7/REQ-NN`.

- [x] REQ-01: one readiness derivation — 635f23c7, 945f9181, b674382e
- [x] REQ-02: no follow-up readiness mutation — 635f23c7, b674382e
- [x] REQ-03: one derivation per issue field — d2acb105, fe9147fa
- [x] REQ-04: in-session lease checks through one check — 905364c1, c2b3cefa, d3c94dc5, b1d7a600
- [x] REQ-05: bulk preview is the apply derivation — 693a2fdd
- [x] REQ-06: archive preview and candidates through the execute planner — 65aafaba, c0775a83, dd95bd9d
- [x] REQ-07: one container-destination request — 4e2bed35
- [x] REQ-08: archive references resolved once — f56bad8d
- [x] REQ-09: one pure configuration assembler — 1ade3e6f
- [x] REQ-10: declarations from captured images — 030df4b1, 42046050, 850eaad3, 86a183c2, a2153a6f, c31c2e57, e5d7bc11
- [x] REQ-11: one pure gate-preset catalog — 1a737bb4
- [x] REQ-12: one evidence capture, inputs resolved once per image — 700a96dd, a869900b, a2153a6f, 6b22ce6d, a8c4223b, 350062e4, c69e07b2, 863fd652
- [x] REQ-13: captured image shared without copying — f7a59f5f
- [x] REQ-14: redo apply opens each parent once — 4d834432
- [x] REQ-15: directory syncs coalesced after interruption tests — 2add77bc, 4d834432
- [x] REQ-16: one no-follow primitive module — 01a88d32, 4334df62, d58f80c9
- [x] REQ-17: one atomic publication function — 1b7f58ae, 7fefd776
- [x] REQ-18: pure, typed domain layer — 5d09e74b, b0dc019f, ceaaf0ab
- [x] REQ-19: snapshot export from one image — 30334066, 82f1647f
- [x] REQ-20: one form per listed primitive — 12064c7b, 219b9df8, 42046050, 4334df62, 635f23c7, 9623fe6a, b0dc019f, b2c28ec1, cfc87a55, e73ad8bc, f6477007, f98f2353
- [x] REQ-21: lock-free reads with admission and retryable conflict (amended to `@/charter/D-25`) — 030df4b1, 1a737bb4, 578ed10d, 65aafaba, bb596228, c0775a83, eaca3f52, d4a20645
- [x] REQ-22: cost reductions evidenced by one harness — 2add77bc, 45d0a971, 6a19e93f, 86a183c2, e7635f2f, 86d76e01
- [x] REQ-23: every redo record appends an event — c517b88b

Stated deviations recorded on their issues:

- `d675102c` REQ-04: an issue listing a dependency twice decodes that dependency twice per image. Filed as v1.1 follow-up `57129f99` under the steward's six-round cap.
- `e5d7bc11` REQ-03: registry-first sources take `file_bytes` semantics (follow-up `74657d61`).
- `86a183c2`: `jit config set` checks the whole declaration set.
- `45d0a971`: the tip record started at load 1.77, under the harness's 2.0 gate.

## Key Decisions

- Every story was re-checkpointed by a criterion-only sol review before closing. A finding inside a criterion became a task under its story, not a follow-up.
- A once-per-image class was closed by enumeration and structural rules rather than instance by instance:
  - per-command declaration memo (86a183c2);
  - registry and document threading from discovery (6b22ce6d, 863fd652);
  - discovery propagates decode failures (c69e07b2).
- Fix-forward after landing: a failed review was fixed with a new commit on a fresh worktree from main, never by rewriting a landed branch.
- When a gate failed only because a worker's required report was missing, the lead committed the report as a linked artifact (863fd652, d4a20645).

## Escalations

All escalations went to the steward (just-in-time-d5) and are resolved. The full list and decisions are in `progress.json` (`escalations`, `decisions`).

- **Rework limits** (f6477007, dd95bd9d, 6a19e93f, e5d7bc11, 86a183c2): one final scoped round each, or a stated deviation (e5d7bc11).
- **Criteria against later rulings:**
  - REQ-21's lock fallback was amended to `@/charter/D-25` in the story and the epic.
  - 693a2fdd's contradiction was settled by ruling.
  - The cited audit list for b2c28ec1 REQ-06 had been deleted upstream, so the steward corrected the criterion.
- **d675102c checkpoint rounds** r2–r6: each remediation task was approved, under a six-round cap.
- **Shared infrastructure:** the user moved every review gate to codex `gpt-6.1-sol` high (be302dc9c).
- **Benchmark method:** the user set scenario (a) to M ∈ {10, 160} before any tip record.
- **Claim synchronization** (b1d7a600): the ruling was revised after the lead showed the claims lock spans the publish.

## Issues Discovered During Execution

Wave numbers are not recorded; issues are listed by the review that surfaced them.

- d675102c checkpoints: e5d7bc11, 6b22ce6d, a8c4223b, 350062e4, c69e07b2 and 863fd652, the remediation of the once-per-image class.
- Epic gate reconciliation: d4a20645 (a retryable conflict surfaced under a configuration-read message).
- Holistic-review run 1: b1d7a600 (asset rescans bypassed the lease check) and 86d76e01 (graph-rule smoke recorded as raw evidence).
- v1.1 follow-ups: 57129f99 (duplicated dependency decode) and 74657d61 (non-regular occupants in captured reads).

## Holistic Quality Notes

- One-resolution properties are owned by two tests: `test_gate_evaluation_resolves_its_inputs_once_per_captured_image` and `test_configuration_parse_parses_each_distinct_source_once_across_captures`. Thread-local counters at each resolution point back them, and each case fails on its parent.
- Cost evidence is reproducible: `dev/benchmarks/cost-evidence-45d0a971/validate.py` rederives the four tip verdicts and the graph-rule smoke from `raw/`.
- Review rounds repeatedly found instances of one defect class one at a time. Briefs that require an enumerated sweep table, committed as evidence, ended that pattern.
