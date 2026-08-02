# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 6

**Date:** 2026-08-02
**Session number:** 6
**Prior handoffs:** `handoff.md` (session 1), `handoff-2.md` (session 2), `handoff-3.md` (session 3),
`handoff-4.md` (session 5), same directory. Every trap in every prior handoff remains in force
unless a trap here records its resolution.

## Current state

- Epic `e204e63d` — state: backlog, claimed by nobody. 77 issues carry the epic label:
  **41 done** (27 at session start), 33 backlog, 3 in_progress.
- **Wave 1 is closed.** All ten taxonomy adoptions, their two roots (`06b736c7`, `ae435979`'s
  successors) and the wave-0 tail are done.
- The three still `in_progress`:
  - `32779829` — the new gate-verdict provenance issue. Merged on main; its gates were running
    when this was written. **Read them before anything else.**
  - `6c47b99a` — deliberately open, see the resequencing ruling below.
  - `6f8f02ba` — holds no work of its own; closes behind `daddfc0b`.
- Progress file: `progress.json`, same directory — per-issue status, gate history, owner rulings,
  surfaced pitfalls.

## What this session did

- **Landed wave 1 in full.** Fourteen issues merged and gated: `c7058cac`, the ten adoptions,
  `06b736c7`'s convergence rework, `be918f22`, `0cf1f351`. `HierarchyTemplate::default()` call
  sites went **43 → 4**, which is the property that lets `daddfc0b` delete the type without
  carrying the rewrite in its own diff.
- **Filed and delivered `32779829` on an owner ruling.** A fifteen-issue wave over one unchanged
  tree spent ~112 minutes re-deriving one whole-workspace verdict. A gate now declares the
  repository inputs its checker reads, a run digests them, and an evaluation over an unchanged
  digest takes the prior verdict instead of executing. **Measured in production:** `06b736c7`'s
  `cargo-ci` reused `32779829`'s verdict in **0 ms against 411 s executed**, the record carrying
  `origin.derivation: "reused"` and naming the source run. Pipeline 4 gated fourteen issues in
  ~50 minutes.
- **Unified the staleness rule** (`32779829` REQ-11/REQ-12, added mid-flight by the owner).
  `install-jit.sh` decided dirtiness from `git status -- . ':(exclude).jit'` while the inventory
  of what actually feeds the binary lived in `build_provenance.rs`, unconsulted. The inventory is
  now one data file, `crates/jit/src/domain/binary_build_inputs.txt`, which Rust `include_str!`s
  and the installer reads as git pathspecs, with a conformance test holding both matchers to
  selecting the same files.
- **Six reworks driven to passing:** `06b736c7` (96 vocabulary literals), `76b16af0` twice,
  `60c211d6`, `a603f02d`, `c7058cac`, `0cf1f351` twice, `1cfb66ff` and `ffe6de33`.

## What to do next

- [ ] Read `32779829`'s gate verdicts (`jit gate status-all 32779829`). Close it if they passed.
      It is the predecessor of the five issues the rest of the epic sits behind.
- [ ] **Re-run `6c47b99a`'s `code-review` only after `8371bd46` lands.** Owner ruling: pull that
      chain forward — `32779829` → `5d12a79f` → `8371bd46` — then re-review. Do not rework
      `6c47b99a` itself; its five criteria cover recording an origin and it meets them.
- [ ] Wave 2 is the five issues now unblocked behind `32779829`: `7d038e97`, `4873b32f`,
      `0db28190`, `e4ba28c5`, `5d12a79f`. Everything in layers 2–7 is transitively behind them.
- [ ] `fd61b44f` (the graceful-shutdown drain test) is `ready` and still undispatched, for the
      third session running. It edits `graceful_shutdown_tests.rs`, which `a53a6f09` has now
      finished with, so the conflict that held it back is gone. Its REQ-03 wants consecutive
      whole-workspace runs; dispatch it when the pipeline is quiet.
- [ ] File the two dogfooding-friction items, still unfiled after two sessions: `jit issue create`
      has no `--description-file` (unlike `jit issue update`), and `jit gate` has no way to
      evaluate one gate across several issues in one command — `32779829` removed the *cost* of
      that, not the ergonomics.

## Traps — do not repeat these

All prior traps remain in force; read every earlier trap section. New this session:

- **Do NOT choose a reading when two criteria conflict — escalate, even at dispatch time.**
  Session 3's handoff says an unsatisfiable criterion is an escalation, not a reading to choose.
  The lead read that trap, understood it as applying to lead review, and then chose a reading of
  the adoption issues' REQ-02/REQ-03 conflict while composing ten dispatch briefs. The gate ruled
  the other way and `1cfb66ff` and `ffe6de33` each lost a review round. A conflict noticed while
  briefing is the same escalation as one noticed while reviewing.
- **Do NOT assume a brief's own instructions are sound when a finding lands on them.** The lead's
  brief for `32779829` said ignored files must never enter a gate's input digest. That made the
  implementation use `git ls-files --exclude-standard`, which silently drops `web/node_modules` —
  which `npm-ci` genuinely reads — so a dependency change would move that checker's verdict
  without moving the digest. The reviewer caught it. When a finding lands on something the brief
  told the worker to do, review the brief, not the worker.
- **Do NOT expect a namespace or prompt amendment to bind an adversarial reviewer generally.**
  Sharpening `[namespaces.per]` and then `contrib/gates/code-review-prompt.md` fixed `39c34568`'s
  label over-reach. It did not fix `6c47b99a`'s: that reviewer simply stopped citing the label and
  cited the decision text directly. **Resequencing — landing the issue that actually delivers the
  decision — is what has worked, both times.**
- **Do NOT edit a live consumer while a gate pipeline is running.** `contrib/gates/` is not a
  build input, but its packaged mirror under `profiles/jit-dogfood/assets/live/` is, and the
  drift assertion requires editing both. Doing so mid-pipeline refused two of `c7058cac`'s gates
  with exit 10. Make those edits between pipelines.
- **Do NOT start a pipeline with any uncommitted file, including `dev/`.** An uncommitted
  `progress.json` made `install-jit.sh` record `dirty=true` and eleven gates refused with exit 10
  in under a second. This is the third time this shape has cost a pipeline across two sessions.
  `32779829` REQ-11 fixes the underlying rule; until you have confirmed that landed, commit first.
- **Do NOT trust `cargo test --workspace --no-run` as the pre-merge check.** Session 5's handoff
  proposed it to close the `verify-commit-builds.sh` gap. It closes the compile half only:
  `c7058cac` and `6c47b99a` merged into a tree that compiled and whose profile acceptance test
  panicked at run time. Run the suite, not the build.
- **Do NOT clear incremental state at `-maxdepth 2`.** Tests create nested target dirs
  (`target/init-commands/debug/incremental`, `target/jit-stale-child-test-cache/debug/incremental`)
  that `cargo-ci`'s post-run check finds and fails on. The session copy of `merge-and-gate.sh`
  sweeps at `-maxdepth 4`.
- **Do NOT accept a worker's "unrelated baseline failure" without a base-commit control.**
  `60c211d6` changed a `pub(crate)` helper that a sibling module in the same target imports, ran
  only its own module, and reported the sibling's failure that way. It failed `cargo-ci` on main.
  Briefs now require the whole target plus a control before any baseline claim.
- **Do NOT read a worker's invariant citation as settled.** `0cf1f351` cited
  `@/invariant/convention-convergence` as its reason NOT to change a shared method's matching
  base. That invariant prescribes the opposite — change the shared form at its source. Reading it
  correctly turned a deferral into a two-commit fix.
- **Do measure a footprint before dispatch; the issue Notes' counts are advisory.** `76b16af0`'s
  Notes say "four call sites across three modules". A read-only `codex exec -m gpt-5.3-codex-spark`
  pass identified them exactly; the first worker instead took two different modules, and its edit
  to `commands/profile.rs` destroyed the subject of `c7058cac`'s scaffold-comparison test.
- **Do NOT expect the Opus workers to answer a request for a report.** Several went idle without
  replying and had to be prodded, twice each; one had committed work the lead had read as absent.
  They act on instructions reliably. Verify their output by reading the branch, and treat the
  report as a bonus.

## Open questions needing invoker input

None blocking. One standing item, now materially changed: gate throughput was the epic's critical
path for three sessions, and `32779829` has removed the bulk of it. `code-review` at ~3 minutes
per issue is now the dominant per-issue cost, and it is genuine per-issue work — it reads each
issue's own `jit:<id>` commits — so it should not be optimised away.

## Reference artefacts

- Epic: `jit issue show e204e63d` — `D-7` … `D-23`, nine live criteria (REQ-06 retired).
- Plan: `e204e63d-plan.md`; manifest: `e204e63d-breakdown.json` — authoritative, but note
  `32779829` was added to the graph this session on an owner ruling and is not in the manifest.
- Boundary audit: `dev/active/7cbefe7c/findings.md` — **S4's reachability claim is disproven.**
- Profile-extraction investigation: `dev/active/a62d444d/findings.md` — predates `D-13`.
- Session tooling (scratchpad, not committed): `merge-and-gate.sh`, `dispatch-codex.sh`,
  `mkbrief.py`, `briefs/`, `logs/`, `discarded-wip-sha.txt`.
