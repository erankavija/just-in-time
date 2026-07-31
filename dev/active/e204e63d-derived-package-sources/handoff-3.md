# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 3

**Date:** 2026-07-31
**Session number:** 3
**Prior handoffs:** `handoff.md` (session 1), `handoff-2.md` (session 2), same directory. Every trap
in both remains in force except where a trap below records its resolution.

## Current state

- Epic: `e204e63d` — state: backlog. Not claimed (the lead did not renew a claim this session).
- Wave in progress: **wave 0 of 10.** All twelve wave-0 roots are implemented, reviewed and merged
  to `main`; their gates were still running when this handoff was written.
- Children summary: 75 issues carry the epic label — 14 done at session start, 12 more merged and
  awaiting gate verdicts, 49 backlog, 7 rejected (no membership labels, by design).
- Active claims: the twelve wave-0 issues are claimed `agent:worker` and are `in_progress`. They
  become `done` when their gates pass.
- Open escalations: none awaiting an answer. One was raised and answered this session (9bdf8025's
  REQ-03, below).
- Progress file: `progress.json` in this directory — per-issue review verdicts, gate history,
  owner rulings, surfaced pitfalls.

## What just happened

- **Wave 0 dispatched in full: twelve issues, two batches.** The two hardest leaves (`a30d704d`
  three-site taxonomy-default removal, `09aea8b3` owned-bytes package reader) went to Opus agents;
  the other ten to `codex exec -m gpt-5.6-luna -c model_reasoning_effort=high`.
- Lead review PASS on first submission: `3e340587`, `0439da41`, `9bdf8025`, `ae435979`, `5dd0df5b`,
  `510297e4`, `f6c23c09`, `4c40e165`, `28e1c647`, `09aea8b3`.
- `fdae1023` FAILED lead review (rework 1, then PASS): REQ-02 is universal over *every* shipped tool
  description, and the guard checked three hand-listed keys. Now scans `SERVER_INSTRUCTIONS`, all of
  `CURATION.include`, and every generated `tools/list` description, deriving the kind vocabulary
  from the repository's own config. Worker verified it fires when `@/inv/dag-acyclic` is reinjected.
- `9bdf8025` PASSED lead review, then **FAILED its `code-review` gate** on REQ-03. Escalated; owner
  ruled REQ-03 is scoped to the graph half. Criterion amended on the issue. See traps.
- **Two cross-branch regressions caught by `cargo-ci` on merged main**, both fixture-only, both
  fixed and merged:
  - `510297e4` — deriving the bracket label from the template made a template declaring none an
    error at apply (REQ-03 requires exactly that). Five fixtures relied on the retired literal.
  - `28e1c647` — a publication-retry fixture declared a partial `[item_kinds.invariant]` table and
    relied on the deleted defaults. **This falsifies that issue's own premise** (see traps).
- `09aea8b3` renamed `EmbeddedProfilePackage` → `ProfilePackage` and `MAX_EMBEDDED_PROFILE_*` →
  `MAX_PROFILE_PACKAGE_*`. Accepted as in-scope; `d913a510` keeps the command-name and help-text
  half. Worker merged main into its branch to integrate with `5dd0df5b` and `4c40e165` on
  `package.rs`; the lead verified all three writers' symbols survive.
- `npm-ci` (9bdf8025) and `mcp-ci` (fdae1023) passed. Ten merged worktrees reclaimed.

## What to do next

- [ ] Read the tail of `merge-and-gate.sh`'s log (path in `progress.json`) or run
      `jit gate status-all <id>` for each wave-0 issue. For every issue whose gates passed:
      `jit issue update <id> --state done`, then commit `.jit`.
- [ ] `a30d704d` is committed on `worktree-agent-a30d704d` (4 commits, 41 files) and **not yet
      merged**. It has doc edits that conflict with merged work in three files (`git merge-tree`
      reports "changed in both" for `docs/reference/configuration.md`, `example-config.toml`, and
      one more). Merge it, resolve, gate it, then close.
- [ ] Consider adding `docs-mechanical` to `a30d704d`: it edits three `docs/reference/` pages and
      carries only `cargo-ci`, `code-review`, `jit-validate`. Adding a gate to a child is an
      autonomous lead action.
- [ ] **CHANGELOG reconciliation.** Nine of wave 0's twelve changes are adopter-visible and only
      `09aea8b3` wrote a CHANGELOG entry. Session 1 recorded a standing decision to reconcile the
      container's CHANGELOG once at epic close; that decision predates the scope quadrupling.
      Decide deliberately whether it still holds, and check what the `code-review` verdicts said
      about it before doing a bulk pass.
- [ ] Wave 1 is 14 issues: the 11 taxonomy-fixture adoptions (all depend on `ae435979` alone),
      plus `39c34568`, `c7058cac`, `f9cab4c2`, `6c47b99a`. Dispatch with
      `scratchpad/dispatch-wave.sh <ids...>` — it claims, creates worktrees, briefs and dispatches
      in one command. **It will refuse until wave 0's issues are `done`**, which is correct.
- [ ] Verify `28e1c647` needs a CHANGELOG entry: it is an adopter-visible behaviour change, not the
      pure cutover its description claims.

## Traps — do not repeat these

All session-1 and session-2 traps remain in force; read both trap sections. The flock/`cargo-ci.sh`,
dirty-install, three-dot-diff, `pgrep`, membership-residue and worker-deferral traps all still
apply. New this session:

- **Do NOT let codex CLI workers commit — they cannot.** `codex-cli 0.146.0` forces `.git`
  read-only under `-s workspace-write`. Neither `-c sandbox_workspace_write.writable_roots` nor
  `--add-dir` lifts it; reproduced on a throwaway repo in `/tmp`, so it is not a worktree artifact.
  The lead commits each worker's worktree diff onto its branch with the `jit:<short-id>`
  attribution the `code-review` gate reads. `--add-dir` DOES work for non-`.git` paths, and
  `$HOME/.cargo` must be granted or cargo cannot take its package-cache lock.
- **Do NOT trust a Node test result from inside the codex sandbox.** `spawnSync` reports a spurious
  `EPERM` while the child actually runs and returns correct output, so `mcp-server`'s suite aborts.
  Run Node suites yourself, or send Node-heavy issues to an unsandboxed agent.
- **Do NOT tell a worker "run every command from your worktree root" and stop there.** A shell `cd`
  does not protect a FILE-EDITING tool: a relative path in an edit resolves against the parent
  checkout. The `09aea8b3` worker wrote its conflict resolution into main's own `package.rs` that
  way. The brief template now states the rule for editing tools separately and requires the `cd`
  inside every cargo command. Run `check-leak-into-main.sh` after every wave regardless.
- **Do NOT merge anything while a gate is evaluating.** Merging `09aea8b3` moved HEAD under a
  running `code-review`, and the stale-binary guard correctly refused it with exit 10 — a wasted
  run. Merges and gate runs must be serialised against each other, not only gates against gates.
- **Do NOT run any debug cargo build in the parent checkout.** `cargo-ci`'s incremental preflight
  fails in ~250ms for EVERY issue in the wave if `target/*/incremental` is non-empty; it cost one
  full gate cycle before it was understood. The gate pipeline now clears it and exports
  `CARGO_INCREMENTAL=0`. The original writer was never attributed.
- **Do NOT argue past a criteria conflict you notice in lead review.** On `9bdf8025` the lead saw
  that REQ-02 (colours derived from the served namespace list), REQ-03 (default taxonomy renders as
  today) and REQ-04 (no namespace literal) cannot all hold — any derivation reading only the list
  produces different colours — and passed the issue on the reading that REQ-03 meant the graph. The
  `code-review` gate failed it on exactly that point. The protocol's No-argue rule applies to the
  lead's own review too: an unsatisfiable criterion is an escalation, not a reading to choose.
  Owner amended REQ-03; the delivered code then passed unchanged.
- **Do NOT trust `28e1c647`'s "unreachable" premise, or the audit finding behind it.**
  `dev/active/7cbefe7c/findings.md` S4 and the issue's own Background claim configuration loading
  rejects a partial `[item_kinds.X]` table, so deleting the three defaults "changes no byte any
  adopter receives". False: `template_apply_atomicity_tests.rs:604` writes a table declaring only
  `scope`/`source`/`source-of-truth`, it loads, and it previously received the defaults. The
  deletion is a real adopter-visible behaviour change. The change is still correct — REQ-02
  requires reporting the missing field — but anything downstream that relied on "pure cutover"
  (CHANGELOG treatment, migration notes, the holistic review) must be revisited.
- **Do NOT assume `scripts/verify-commit-builds.sh` is verifying anything.** It reports a cold
  `cargo build --workspace` over a fresh `git archive` extraction as passing in 24–30 seconds, with
  no sccache, no `RUSTC_WRAPPER`, no `VERIFY_COMMIT_TARGET_DIR`, and an empty cache dir. That is
  not a real build of this workspace's 260 dependencies. Merge-commit integrity is independently
  evidenced by `cargo-ci` (which demonstrably catches real defects — it caught both regressions
  above), so the wave was not run blind, but the guard itself is suspect. File it.
- **Do NOT use `set -- $var` in a helper script.** The harness shell is zsh, which does not
  word-split unquoted parameters; a gate loop silently passed `"9bdf8025 npm-ci"` as one argument
  and four evaluations no-opped with "Issue not found".

## Open questions needing invoker input

None blocking. Two standing items:

- The container remains very large for one `holistic-review` (72 manifest entries, 10 waves,
  7 stories). Splitting was offered and declined twice (`D-10`). Raise again only if the epic gate
  proves unworkable.
- Gate throughput is the epic's critical path, not worker throughput. Gate evaluations must run
  strictly sequentially (concurrent ones lose results to per-issue locks) and `cargo-ci` is ~7
  minutes each. With ~49 issues left, gate time alone is on the order of ten hours. If that is
  unacceptable, the question for the owner is whether `cargo-ci` can be scoped per footprint rather
  than run whole-workspace per issue — that is a gate-definition change and therefore an escalation.

## Reference artefacts

- Epic: `jit issue show e204e63d` — `D-7` … `D-23`, nine live criteria (REQ-06 retired).
- Plan: `dev/active/e204e63d-derived-package-sources/e204e63d-plan.md` — passed plan-review round 5.
- Manifest: `dev/active/e204e63d-derived-package-sources/e204e63d-breakdown.json` — authoritative.
- Boundary audit: `dev/active/7cbefe7c/findings.md` — cited `A<n>.<m>`. **S4's reachability claim is
  disproven; check its other claims before relying on them.**
- Profile-extraction investigation: `dev/active/a62d444d/findings.md` — cited `F<n>.<m>`; predates
  `D-13`.
- Container investigation: `dev/active/e204e63d-derived-package-sources/e204e63d-investigation.md` —
  a dated finding record; its packaged-asset counts are already stale and that is not a defect.
- Progress file: `dev/active/e204e63d-derived-package-sources/progress.json`.
