# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 1

**Date:** 2026-07-31
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `e204e63d` — state: backlog (claimed `agent:jit-execution-lead`). **Cannot reach Done on its current criteria** — see Open questions.
- Wave in progress: wave 3 of 3 executable waves. The originally planned waves 4–6 are halted.
- Children summary: 14 done, 0 in_progress, 2 ready but halted (`39c34568`, `3e340587`), 10 backlog and halted, 0 rejected. **All executable work is complete.**
- Active claims: none. Every worker stood down.
- Open escalations: four owner decisions taken this session, all resolved and recorded in `progress.json` under `owner_rulings` and `escalations`. Four **new** questions are open — see below.
- Progress file: `progress.json` in this directory. It carries the re-planned waves, the halted set with per-task disposition, six `surfaced_pitfalls`, and the owner rulings.

## What just happened

- Escalated two plan defects before dispatch; both approved. `779ea308` rewritten to derive build inputs from packaged targets instead of ~60 hand-written path literals. New task `9d451c98` created to retire `scripts/` as a live-source root (4 packaged of 45 tracked, no category separating them) and collapse the byte-identical `scripts/ai-review.sh` / `contrib/gates/ai-review.sh` pair.
- Wave 1 dispatched (5 issues), all done: `9943c488`, `67da1d70`, `c7a548c3`, `9d451c98`, `e1a18372`.
- `e1a18372` failed lead review — its test *wrote* `.jit/templates.toml`, making epic REQ-02 unmeetable and dirtying the tree on every `cargo test`. Reworked to a shell entry point + cargo example + assert-only suite.
- The `e1a18372` worker then surfaced that the lead's verdict ("must not be a `#[test]`") created a second convention against six incumbent `#[ignore]`d regenerate-twins. Owner ruled convergence mandatory inside this epic → `65ff0f38` created.
- Owner ruled the dogfood profile must leave the binary (`@/invariant/domain-agnostic` has no durable exception). Investigation `a62d444d` produced `dev/active/a62d444d/findings.md` (44 KB) and is done.
- Story `cc75b4e6` halted on that ruling; 3 tasks superseded, 9 survive retargeted.
- `229e7389` created and completed — the extraction's one strict ordering constraint (`jit validate` no longer loads a profile package unconditionally).
- Wave 2 done: `f77fd51b` (shipped-policy freshness check, M7) and `097b9f48` (parsed-declaration drift guard). Both failed `code-review` once and passed on rework.
- Stories `25bdda50` and `6f8f02ba` completed with all gates.
- Lead-direct fix (owner-directed): `install-jit.sh` no longer counts tracker-data churn as build dirt.
- `65ff0f38` done after two rework rounds. Round 1 (lead review): a retired invocation rendered into `docs/reference/error-codes.md`'s header from a Rust string literal. Round 2 (`code-review`): the shipped-policy generator reported post-render failures as 2 while its seven siblings report 1; resolved by auditing all 18 failure sites (13 stay 2, 4 move to 1) with the boundary at "was the classification in hand".

## What to do next

- [ ] Put the four Open questions to the owner. **Nothing in this epic is executable until they are answered** — every unblocked issue is done.
- [ ] Do not dispatch any halted `cc75b4e6` task. `progress.json.halted` records which are superseded and which survive retargeted.
- [ ] Do not dispatch any halted `cc75b4e6` task. `progress.json.halted` records which are superseded and which survive retargeted.

## Traps — do not repeat these

- **Do NOT dispatch a rework agent into a worktree whose original worker is still alive.** Done this session with `e1a18372`: the original worker woke, both edited `crates/jit/src/profile/dogfood.rs`, and the file carried two parallel implementations of the same render. Cost a stand-down negotiation and a cleanup list. One worktree, one writer. If the original is unresponsive, confirm it is finished before dispatching a replacement.
- **Do NOT tell workers to run `./scripts/cargo-ci.sh`.** This host serialises every build behind `flock /tmp/jit-cargo-ci.lock`. Wave 1's dispatch prompt said to run it and put four builds in one queue — one of them verifying a design already rejected — stalling the wave ~45 minutes. Workers run targeted checks only (`cargo test -p jit --lib <module>`, `cargo clippy`, `cargo fmt`, `jit validate`); the lead runs the authoritative gate on `main` after merge.
- **Do NOT run `./scripts/install-jit.sh` with a dirty working tree.** `install-jit.sh` records the dirty flag, and `assess_binary_provenance` (`crates/jit/src/domain/build_provenance.rs:181`) treats a dirty build as **unconditionally stale for the life of the binary** — every subsequent gate refuses. Cost two wasted reinstalls. Tracker-data churn no longer counts (fixed this session, commit `af1e38f6`), but any other dirt still does.
- **Do NOT assume a merge leaves the installed binary current.** `crates/jit/src/**` and `profiles/jit-dogfood/**` are declared build inputs. After merging anything touching them, reinstall before running gates, or gates exit 10 with `STALE_BINARY`. This is now sharper: `f77fd51b`'s M7 check makes `docs-mechanical` fail with exit 2 on a stale binary, where previously a manual run tolerated it.
- **Do NOT grep with loose patterns across the repo for stale-narrative sweeps.** `dev/archive/**` contains vendored minified JS and `.jit/gate-runs/**` contains full test-output blobs; both produce megabytes of false matches. Scope sweeps to `crates docs scripts dev/index.md AGENTS.md CLAUDE.md` and exclude `dev/archive`.
- **Do NOT trust `git diff --stat main..HEAD` on a worker branch.** Two-dot diff against a moved `main` renders main's own commits as apparent reverts by the worker. Use `main...HEAD` (three-dot). This wrongly implicated a worker in reverting `.jit/` state this session.
- **Do NOT poll for worker completion with a `pgrep -f` wait loop.** The `-f` pattern matches the wrapper shell running the loop, so it never exits. The harness blocks it. Poll the actual condition, or use a background command that exits on it.
- **Do NOT merge a branch before its `code-review` gate has run** unless you accept the fix landing as a follow-up commit. Done three times this session (`097b9f48`, `f77fd51b`, and nearly `65ff0f38`); each rework then had to land on top of already-merged work. The alternative serialises the wave behind the reviewer, so this is a real trade-off, not a pure error — but decide it deliberately.
- **Do NOT treat a worker's "left undone / outside the criteria" note as safely deferred.** Twice this session the adversarial `code-review` gate raised exactly the item the worker had flagged and the lead had accepted (`9d451c98`'s investigation reference, `097b9f48`'s whole-registry comparison). When a worker flags a decision point, resolve it against the contract before running the gate.
- **Do NOT search for stale text only in markdown.** `65ff0f38`'s defect was a retired command inside a Rust string literal that *renders into* a generated markdown page. Sweep the renderers, not just the rendered output.

## Open questions needing invoker input

- Question: What should epic REQ-04 and REQ-06 say?
  - Context: Both name the binary ("two builds **embed** byte-identical package content"; "editing a live consumer reports a **binary** stale") and lose their referent once the profile leaves the binary. REQ-03 and REQ-05 survive unchanged and gain value (`findings.md` §8).
  - Options: (A) restate REQ-04 over the produced tree and close REQ-06 as covered by REQ-03; (B) drop both and re-scope the epic; (C) leave and mark the epic partially delivered.
  - Recommendation: A. The reproducibility property is still worth having, measured over the package hash rather than the embed.

- Question: How does `jit validate --fix` re-resolve a package on a later run, given no package bytes live in `.jit/`?
  - Context: The owner ruled contributed profiles are not copied into `.jit/`. `findings.md` §10.2 shows repair can then only recompute while the package resolves. `229e7389` deliberately fails loudly rather than degrading, so nothing is foreclosed. Must be settled before extraction step 2 builds the resolver.
  - Options: (A) the provenance record at `.jit/profiles/<id>.json` carries the source path and repair re-resolves from it; (B) `--from` only, accepting that repair degrades to a hard failure when the package is absent.
  - Recommendation: A — it preserves the capability without putting package bytes in `.jit/`.

- Question: Approve the narrow `@/charter/D-8` amendment?
  - Context: D-8 says "one **embedded**, offline profile" and defers local packages to post-1.0; the ruling reopens it (`findings.md` §9). An unamended charter lets the next container read the reopening as permission for the whole profile lifecycle.
  - Recommendation: amend narrowly to "one offline profile discovered from a declared location", leaving the rest of the deferred list intact.

- Question: Scope and sequence extraction steps 2–5?
  - Context: `findings.md` §5.3 gives the ordering: discovery + owned bytes + `VersionReq` (2), publish and install the artefact (3), delete the embed and the preset trio together — indivisible (4), regenerate `gate-presets.md` + amend the invariant (5). Step 1 is done. This is a larger body of work than the epic's remaining criteria describe.
  - Options: (A) new tasks inside e204e63d; (B) a new epic; (C) fold into an existing milestone container.
  - Recommendation: B — it is a distinct deliverable with its own criteria, and e204e63d's criteria describe packaging, not extraction.

## Reference artefacts

- Epic: `jit issue show e204e63d`
- Plan: `dev/active/e204e63d-derived-package-sources/e204e63d-plan.md` (carries an "Owner-approved amendments during execution" section)
- Prior investigation: `dev/active/e204e63d-derived-package-sources/e204e63d-investigation.md`
- Breakdown manifest: `dev/active/e204e63d-derived-package-sources/e204e63d-breakdown.json`
- **Profile-extraction findings:** `dev/active/a62d444d/findings.md` — the authority for everything halted. §2.1 gives the criterion separating engine capability from a specific application; §5.3 the ordering; §8 the per-criterion verdict on this epic; §10 the costs with no route back.
- Progress file: `dev/active/e204e63d-derived-package-sources/progress.json`
