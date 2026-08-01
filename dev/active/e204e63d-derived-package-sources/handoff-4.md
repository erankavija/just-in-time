# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 5

**Date:** 2026-08-02
**Session number:** 5
**Prior handoffs:** `handoff.md` (session 1), `handoff-2.md` (session 2), `handoff-3.md` (session 3),
same directory. Session 4 ended without one; what it did is reconstructed below. Every trap in
every prior handoff remains in force unless a trap here records its resolution.

## Current state

- Epic `e204e63d` — state: backlog, unclaimed. 76 issues carry the epic label: **27 done**,
  34 backlog, 15 in_progress.
- **Wave 0 is closed.** All twelve roots are done. `5dd0df5b` was the last, and closed on the
  owner's label-authority ruling below.
- **Wave 1 is mid-flight.** Of the fifteen dispatched: `f9cab4c2` is done; `39c34568`, `06b736c7`
  and `6c47b99a` are merged with gates re-running at the time of writing; `c7058cac` is reviewed
  and NOT yet merged; the ten taxonomy adoptions are blocked behind `06b736c7` and must be
  re-dispatched as reworks.
- `fd61b44f` (the graceful-shutdown drain test, an epic dependency outside the epic label) is
  `ready` and deliberately undispatched — see below.
- Progress file: `progress.json`, same directory — per-issue status, gate history, owner rulings,
  surfaced pitfalls, escalations.

## What session 4 did (reconstructed — it left no handoff)

From `git log`, `.jit` state and the gate-run records: merged and gated the wave-0 tail
(`a30d704d`, then reworks for `09aea8b3` and `510297e4`); fixed the graceful-shutdown drain test
twice, then marked it `#[ignore]`d against the new issue `fd61b44f`; amended
`[namespaces.satisfies]` so the label denotes contribution. It left `09aea8b3` and `510297e4`
fully gated but unclosed, and `5dd0df5b` failing `code-review`.

## What this session did

- **Closed wave 0.** `09aea8b3` and `510297e4` were gate-passed and unclosed; closed both.
  `5dd0df5b` had failed `code-review` twice on the same finding — its `satisfies:REQ-08` label
  "claims credit for discovery and application" that the issue defers. Session 4's fix (amending
  the namespace description) did not bind the reviewer. Owner ruled: cite the namespace
  declaration as a label's authority in AGENTS.md. One bullet added to
  `profiles/jit-dogfood/assets/regions/agents-jit-guidance.md` and its AGENTS.md region
  (`c250dbcf`); the gate passed on the first re-run.
- **Dispatched wave 1** — fifteen issues on base `da174a6b`. Ten mechanical taxonomy adoptions to
  codex `gpt-5.6-luna`; five leaves carrying design judgement to Opus agents.
- **Found the fixture split, mid-wave.** The ten adoptions are contracted to consume ae435979's
  fixture "as authored", but it writes the whole `.jit/config.toml` from its own vocabulary and
  its renderer is private, so a suite declaring item kinds beside its taxonomy cannot use it.
  Both early workers reached for `declared_test_taxonomy()` — the parallel helper session 3's
  handoff had already flagged. Owner ruled: converge first, rework after. Task `06b736c7` created,
  wired between `ae435979` and all ten adoptions, and delivered: one declaration at
  `crates/jit/src/test_taxonomy.rs` rendering into three forms, both duplicates retired, ~160
  literal type-name assertions rewritten.
- **Made the invariant kind default vocabulary.** `c7058cac` delivered the D-20-correct five-kind
  package and its own test showed REQ-02 and REQ-04 could not both hold. Owner ruled the
  exclusion was about the scaffolder, not the kind. D-20 amended; `c7058cac` gained REQ-06
  (initialization creates the invariant registry as it creates the gate registry) and REQ-07 (the
  workflow package contributes rather than publishes); reworked and re-reviewed.
- **Landed `f9cab4c2`** (done), and merged `39c34568`, `06b736c7`, `6c47b99a` after fixing two
  cross-branch defects (below).

## What to do next

- [ ] Read the gate verdicts for `39c34568`, `06b736c7`, `6c47b99a` (tail
      `scratchpad/logs/pipeline.log`, or `jit gate status-all <id>`). Close each that passed.
- [ ] Merge and gate `c7058cac`. It is reviewed and PASSES on all seven criteria; its branch
      `worktree-agent-c7058cac` has three commits. **Add `docs-mechanical` to it before gating** —
      it now edits `docs/reference/cli-commands.md` and `docs/reference/storage-format.md`.
- [ ] Re-dispatch the ten adoptions as reworks once `06b736c7` is `done`:
      `1aa2b486 1cfb66ff 60c211d6 76b16af0 889a939a a4f03da3 a53a6f09 a603f02d be918f22 ffe6de33`.
      Tooling is ready: `scratchpad/mkbrief.py <sid> <base> codex <suffix> <addendum>` takes a
      worktree suffix (their plain branch names are taken by preserved WIP) and an addendum file;
      `scratchpad/adoption-addendum.md` is written and names the three fixture forms, which one
      each call site needs, and that assertions read `type_at_level(level)` rather than literals.
      Their measured footprints are in `progress.json` under wave 1.
- [ ] `76b16af0` reported that none of its four call sites asserts on vocabulary, so no change is
      warranted. Re-verify that against the surviving declaration rather than accepting it.
- [ ] Dispatch `fd61b44f`. Held back this session for two reasons: it edits
      `crates/server/tests/server_integration/graceful_shutdown_tests.rs`, which is one of
      `a53a6f09`'s three call sites, and its REQ-03 wants evidence from consecutive
      whole-workspace runs, which contends with every gate on this host. Dispatch it when the
      pipeline is quiet, and let it run `cargo test --workspace` (not `cargo-ci.sh`).
- [ ] File the two dogfooding-friction items when convenient: `jit gate` has no way to share one
      checker run across issues, so ten test-only issues on one HEAD each pay a full
      whole-workspace `cargo-ci`; and `jit issue create` has no `--description-file`, unlike
      `jit issue update`.

## Traps — do not repeat these

All prior traps remain in force; read every earlier trap section. New this session:

- **Do NOT dispatch a wave without reading the prior handoff's trap list against the dispatch
  set.** Session 3's handoff said `a53a6f09` and `76b16af0` "must be briefed to DELETE the
  parallel helpers". The wave-1 briefs did not carry it, and ten workers were dispatched against
  a contract the trap predicted they could not meet. Composing a brief from the issue description
  alone is not enough — the handoff chain is part of the brief's input.
- **Do NOT run a jit or git inspection command without an explicit absolute `cd` to the main
  checkout.** The harness shell's working directory persists between calls. A `cd` into a
  worktree for one `git diff` made the next three commands read that worktree's stale `.jit`, and
  a gate that had actually PASSED was read as failed. `merge-and-gate.sh` already does this
  correctly; ad-hoc inspection is where it bites.
- **Do NOT leave the tree dirty when a pipeline will install the binary.** An uncommitted
  `progress.json` made `install-jit.sh` record `dirty=true`, and the stale-binary guard then
  refused both gates with exit 10 — one wasted merge-and-gate cycle. Commit before any pipeline
  that installs.
- **Do NOT trust `scripts/verify-commit-builds.sh` to catch a merge that breaks test code.** Its
  contract is `cargo build --workspace`, which never compiles `#[cfg(test)]` modules. Merging
  `6c47b99a` (which gave `ProfilePackage::from_files` a second argument) with `39c34568` (which
  added a test helper calling the one-argument form) produced a tree that git merged cleanly, the
  guard passed, and `cargo test -p jit --lib` could not compile. Note this corrects the session-3
  handoff: the guard is not vacuous, it is scoped. `cargo test --workspace --no-run` would close
  the gap.
- **Do NOT pre-check a merge against a HEAD you are about to move.** `git merge-tree` said all
  three branches merged cleanly; they did, against that HEAD. After two of them landed, the third
  conflicted in CHANGELOG.md. Re-check each merge against the HEAD it will actually meet, or
  merge one at a time and read the result.
- **Do add `mcp-ci` to any issue that changes a serialized shape the CLI reports.** `6c47b99a`
  changed `ProfileOrigin` from a bare string to a tagged enum, and
  `mcp-server/test-integration.js:456` asserted the bare form. The gate was added to that issue on
  suspicion and caught it. Its default gate set (cargo-ci + code-review) would not have.
- **Do NOT assume an idle notification means a worker stalled or finished cleanly.** Five Opus
  workers reported idle this session; every one had committed its work and left a clean tree.
  Check `git -C <worktree> log main..HEAD` and `git status --short` before acting on an idle ping.

## Open questions needing invoker input

None blocking. Standing items from session 3 (container size for one `holistic-review`; gate
throughput as the critical path) are unchanged and were not revisited. The gate-throughput
number is now measurable: `cargo-ci` runs about 8 minutes per issue and every issue merged onto
one HEAD re-runs the identical whole-workspace suite.

## Reference artefacts

- Epic: `jit issue show e204e63d` — `D-7` … `D-23` (D-20 amended this session), nine live criteria.
- Plan: `e204e63d-plan.md`; manifest: `e204e63d-breakdown.json` — authoritative.
- Boundary audit: `dev/active/7cbefe7c/findings.md` — **S4's reachability claim is disproven.**
- Profile-extraction investigation: `dev/active/a62d444d/findings.md` — predates `D-13`.
- Session tooling (scratchpad, not committed): `merge-and-gate.sh`, `dispatch-wave.sh`,
  `dispatch-codex.sh`, `mkbrief.py`, `adoption-addendum.md`, `briefs/`, `logs/`.
