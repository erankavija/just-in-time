# Handoff — Version 1.0 release (b80e3c70) — session 1

**Date:** 2026-07-29
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `b80e3c70` — state: backlog, assigned `agent:jit-execution-lead`
- Wave in progress: wave 1, sub-wave 1a of 7 waves — **all 8 workers complete and reported**; nothing merged, gated, or transitioned
- Children summary: 0 done, 8 claimed with delivered branches, 14 backlog/ready, 0 rejected
- Active claims: `7edd2fe8`, `8fa7261b`, `bb302dbd`, `f04f7888`, `f289ff18`, `c7f8ebc7`, `eeee8a1a`, `985ab96d` — all `agent:worker`, claimed 2026-07-29 ~14:12Z
- Open escalations: None.
- Progress file: `progress.json` in this directory

Nothing is merged to `main`. No gate has been evaluated. No issue has been
transitioned. Every worker branch is anchored to `a8182e6f`.

Worker branch state at handoff (`worktree-agent-<short-id>`, worktrees under
`.agents/worktrees/agent-<short-id>`):

| Issue | HEAD | Commits | Reported |
|---|---|---|---|
| 7edd2fe8 | bc0ca679 | 5 | yes |
| 8fa7261b | 45bd85c5 | 2 | yes |
| bb302dbd | 0d9d28ba | 3 | yes |
| f04f7888 | cb64d918 | 4 | yes |
| f289ff18 | f4418c21 | 2 | yes |
| c7f8ebc7 | 977c3894 | 2 | yes |
| eeee8a1a | ac46b11d | 2 | yes |
| 985ab96d | 8d538c93 | 1 | yes |

All eight trees are clean; no worker left uncommitted work.

## What just happened

- Discovered the epic already broken down: 22 children, `jit validate --scope b80e3c70` exits 0, so Section 3 was skipped for Section 4. No planning/breakdown bracket node exists (`jit issue list --label brackets:b80e3c70` returns 0); the decomposition arrived with commit `284371ce`, which replaced rejected epic `8b05a612`.
- Computed 7 topological waves over the 22 children. Wave 1 holds 11 zero-dependency issues; split into 1a (8), 1b (2), 1c (1) on file-footprint collisions.
- Assigned the epic, claimed the eight 1a issues, wrote `progress.json`, committed as `a8182e6f`.
- Created 8 worktrees via the dispatch script, all SHA-verified against `main` HEAD `a8182e6f`.
- Dispatched 8 `general-purpose` workers, each barred from `.jit/` writes, state transitions, and gate evaluation.
- `eeee8a1a` — returned; replaced the `resolve_footprint()` config lookup with the literal `FOOTPRINT=("docs")`. Sent back for a stale-narrative sweep it had declined; it then fixed 5 stale line-range citations in `dev/active/e204e63d-derived-package-sources/e204e63d-investigation.md` and answered the single-source-prose challenge (no config field denotes the adopter surface; `citation_scan_roots` is a whole-repo scan list including `crates/`, and deriving the default from the gate's own override is circular). Answer accepted.
- `8fa7261b` — returned; mermaid 11.12.2→11.16.0, axios 1.13.2→1.18.1, both minor, zero consumer migration, `found 0 vulnerabilities`, no override/`.npmrc`/audit-level escape. Added 4 tests driving the real mermaid module (previously fully stubbed).
- `bb302dbd` — returned; zod 3→4 major plus SDK 1.0.4→1.30.0. Evidenced the advertised protocol surface is byte-identical across both dependency sets (sha256-equal 971,821-byte capture over 112 tools, 37 outputSchemas, and the initialize handshake), and that exactly one assertion in its final suite is version-dependent.
- Lead accepted the zod 4 major (see Decisions below); the verification question it raised is now closed — see the first item under What to do next.
- `985ab96d` — returned; chose removal over wiring, deleting `commands/plan_doc.rs` entirely and relocating `PLAN_DOC_LABEL` beside its only reader. Named the superseding commit for each of the three functions (`b75ef93e`, `936a4c27`, and never-called respectively) and rejected the wiring alternative on three counts. Swept 6 stale references including a dangling citation in another issue's active doc. cargo-ci green, 4053 tests.
- `7edd2fe8` — returned; built a workflow-contract harness (pinned actionlint 1.7.12 + a structural verifier + a 20-case vector suite, 19 of them violation fixtures) and pinned 16 external actions to SHAs resolved live via `git ls-remote`, none guessed. Found and fixed a pre-existing YAML parse defect in `ci.yml` that predates the wave.
- `c7f8ebc7` — returned; one path-segment-boundary rule with no destination special case, so REQ-01 and REQ-04 hold by the same construction rather than two mechanisms. cargo-ci green, 4072 tests.
- `f04f7888` — returned; recorded a real SIGTERM run (exit 0 at 5.0517s, stalled connection force-closed at 5.0016s, SSE EOF at ~226µs) rather than only the asserted bound, with three independent discriminators separating a forced close from a voluntary finish. Folded its real-process test into a new `server_integration` suite instead of a 12th Cargo target, keeping the footprint budget at 11/12.
- `f289ff18` — returned; extracted one shared resolution rule (`document/reference.rs`) that both `jit validate` and `jit doc check-links` now call, so REQ-03's "the two commands agree" holds by construction rather than by coincidence. The three `41aa1b75` references cited in the issue now report 0 errors and validate agrees. cargo-ci green, 4080 tests.

## What to do next

- [x] **Closed.** `bb302dbd`'s default-containment question is answered and verified — do not re-open it. Exactly one advertised default exists across all 112 tools at both dependency sets and all three surfaces (`jit_item_search.query = ""`, optional), so the non-empty-default case that would change CLI argv under zod 4 is empty. A differential sweep over all 112 tools shows 0 whose `(accepted, argv)` differ; the sole delta is `jit_item_search`'s intermediate `resolvedArgs` going `{}` → `{"query": ""}`, absorbed by `buildCliArgs` dropping empty-string positionals (`cli-executor.js:105`). Structural bound: `tool-generator.js` propagates defaults only for positional args (`:48-50`), with no `flag.default` branch in the flag loop (`:59-77`), so a flag default could never reach `validateArguments` at all. `bb302dbd` is ready to merge and review on its evidence.
- [ ] Two `7edd2fe8` judgement calls to weigh at review, both defensible but both wider than the criteria's literal text: it added a top-level `permissions: contents: read` to four workflows that had none (without it the REQ-01 permission assertions would be vacuous), and it pinned `softprops/action-gh-release` to what `@v1` resolves to today rather than bumping to v2, on the grounds that pinning is a fidelity operation and `f9af9788` rewrites `release.yml` anyway. It also declined to add a `workflow-contract` gate to `.jit/gates.toml`, correctly, since that is a lead decision — decide whether the harness should gate issues or stay CI-only.
- [ ] Run `.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh` before any commit on `main`. The dispatch snapshot is `/tmp/lead-pre-dispatch-20260729-141201.txt`.
- [ ] Merge each branch into `main` sequentially with `git merge --no-ff`, running `scripts/verify-commit-builds.sh` after **each** merge before the next one. Three shared-file conflicts are expected, recorded with resolutions in `progress.json` under `merge_conflicts_expected`: `CHANGELOG.md` across `7edd2fe8`/`c7f8ebc7`/`f04f7888` (three additive Unreleased entries — keep all), `docs/reference/cli-commands.md` across `c7f8ebc7`/`f04f7888` and later wave-1b's `ef118aea` (different paragraphs; merge `c7f8ebc7` then `f04f7888`, confirm the moving-path-citation paragraph survived intact, and only then dispatch `ef118aea`), and `f04f7888`'s git-rename of `crates/server/tests/document_api_tests.rs` into `tests/server_integration/` (no other 1a branch touches `crates/server/`, but a rename merges badly against any later branch editing the old path).
- [ ] Evaluate gates **sequentially**, from the main checkout, with an explicit `cwd` — see trap 5.
- [ ] Review each issue through all six tiers of `references/lead-review-protocol.md` before transitioning anything to `done`.
- [ ] `985ab96d` needs two lead actions on `main` after merge, which the worker was barred from doing: `jit doc add 985ab96d dev/active/985ab96d/985ab96d-decision.md`, and mirror its REQ-01 remove-vs-wire decision into the issue as a `## Decisions` item. REQ-03 is only satisfied once the decision is recorded and discoverable.
- [ ] Fix the stale comment in `.jit/gates.toml` under `[gates.checker.env]` above `DOCS_FOOTPRINT = "docs/"`, which asserts the checker's fallback derives from `[documentation].permanent_paths`. `eeee8a1a` makes that false. It is a TOML comment, not the `description` field, so no gate contract changes and `docs/reference/rules-and-gates.md` stays projection-fresh. Commit under `jit:eeee8a1a`.
- [ ] Check `CHANGELOG.md` coverage after the merges. Four branches (`7edd2fe8`, `c7f8ebc7`, `f04f7888`, `f289ff18`) each added an Unreleased entry; `8fa7261b`, `bb302dbd`, `985ab96d`, and `eeee8a1a` each decided against one and stated why (`985ab96d`: nothing adopter-observable, since the removed error arm was unreachable and no exit code moved; `bb302dbd`: left to the lead as the root manifest sits outside its blast radius). Decide whether the two dependency majors — zod 3→4 and the mermaid/axios bumps — warrant an entry, and add it if so.
- [ ] Then dispatch sub-wave 1b (`f9e42a43`, `ef118aea`), then 1c (`a122b9b3`) alone. **Re-read `f9e42a43`'s dispatch assumptions before writing its prompt.** `f289ff18` moved the ground under it: an unpinned document reference naming a directory or symlink now classifies as `missing_document`, and `f9e42a43`'s REQ-03 requires exactly that case to say "not a supported artifact type" rather than "not found". The message now lives in the shared resolver `crates/jit/src/document/reference.rs`, which both `jit validate` and `jit doc check-links` call — so the fix is one edit in one place, but only if `f289ff18` is merged first. Dispatching `f9e42a43` against the pre-merge tree would send it to a code shape that no longer exists. Full detail is in `progress.json` under its `blocking_predecessor_note`.
- [ ] Reclaim wave-1a worktrees once merged (`git worktree remove` only for branches in `git branch --merged main`).

## Traps — do not repeat these

- **Do NOT let a worker write anything under `.jit/`.** Eight worktrees each carry their own `.jit/` copy, and `events.jsonl` plus `index.json` collide on merge — concurrent agents interleave that log and you cannot stage only one issue's lines. This session barred `.jit/` writes outright and had workers name needed state changes in their return summary instead. All three returned branches verified `.jit/` byte-identical to base. Keep the rule for 1b and 1c.
- **Do NOT claim the epic with `jit issue claim`.** `b80e3c70` depends on its own sink child `bb03df0a`, so claiming fails with `Cannot transition to 'in_progress': issue blocked by 1 unmet dependencies`. Use `jit issue assign <id> <assignee>`, which assigns without a state change. The CLI prints this hint itself.
- **Do NOT dispatch all 11 wave-1 issues together.** Three collisions exist: `f9e42a43` and `f289ff18` both edit `crates/jit/src/commands/document.rs` and `crates/jit/tests/cli_query_graph/check_links_tests.rs`; `ef118aea` and `c7f8ebc7` both edit the archival citation planner (`domain/artifact_plan.rs`, `storage/artifact_planning.rs`); `a122b9b3` adds whole-tree invariant enforcement and must fix every violation the enforced tree reports, so it collides with all of them. That is why 1b and 1c exist. The footprint rationale is recorded per sub-wave in `progress.json`.
- **Do NOT read an idle notification as a finished worker, and do NOT re-dispatch on one.** `8fa7261b` and `bb302dbd` both pinged idle with committed work and no report; both produced complete reports when asked directly. Check `git -C .agents/worktrees/agent-<id> log --oneline a8182e6f..HEAD` and `git status --porcelain` before concluding anything about a worker's state.
- **Do NOT evaluate gates in parallel or from a drifting working directory.** Parallel `jit gate evaluate` calls lose results to per-issue locks, and a persistent `cwd` has previously written gate evidence into a different worktree's `.jit`. Run them one at a time with the cwd stated explicitly on each invocation.
- **Do NOT accept a worker's "another worktree owns that file" claim without checking.** `w-eeee8a1a` declined to fix stale text in `dev/active/e204e63d-derived-package-sources/e204e63d-investigation.md` on the belief that issue `e204e63d` held it. No worktree exists for `e204e63d` — `git worktree list` shows only this wave's 8 plus `lead-install-clean` and `steward-v1-readiness`. Verify, then direct the fix.
- **Do NOT use `jit issue status --full`.** That flag does not exist on the command; the call fails and returns no JSON. Use `jit issue show <id> --json` per issue. `jit issue status <ids>... --json` is real but emits only `{short_id, state, gates:[{key,status}], unmet_dependencies, title}`.
- **Do NOT expect `dev/plans`, `dev/sessions`, `dev/design`, or `dev/experiments` to exist in a worktree.** They are empty on `main` and therefore absent from every worktree; a citation checker pointed at them reports MISSING there while passing on `main`. Not a bug to chase.
- **Do NOT let two workers write the same scratchpad filename.** `f04f7888` lost its first `cargo-ci` output because another worker redirected to the same `<scratchpad>/cargo-ci.log`. Namespace scratchpad files per issue in the dispatch prompt (`cargo-ci-<short-id>.log`).
- **Expect `cargo-ci` runs to serialize on a host-wide lock.** Workers reported queueing 4–10 minutes behind another worktree's run. That is contention, not a hang — do not re-dispatch on it. It also means a worker may commit a docs-only change while its run is still queued, so confirm which tree a reported run actually covered before treating it as evidence for the final commit. The lead re-runs every gate on `main` after merge regardless, which is what settles it.
- **Do NOT set a worker loose without `CARGO_INCREMENTAL=0`.** `cargo-ci` fails on leftover `target/debug/incremental` directories. Every Rust dispatch prompt this session carried the constraint.

## Open questions needing invoker input

None blocking. One lead decision recorded for visibility rather than approval:

- **zod 3 → 4 major in `mcp-server/`, accepted.** `bb302dbd`'s declared `^3.24.1` floors below SDK 1.30's peer range `^3.25 || ^4.0`, so the range had to move regardless of the audit; zod 3.x stopped shipping (last stable 3.25.76, 2025-07-08) while 4.x is current, so under `@/charter/D-11` a later zod-3 advisory would have had no in-range fix. The issue's REQ-03 explicitly contemplates a validation-API migration, so this sits inside the issue's contract rather than outside it. The worker offered to revert to `^3.25.76`; declined. Reversible on its branch if you disagree.

## Surfaced pitfalls (reconcile against epic criteria before the epic's own gates)

- 11 **dev-only** high advisories remain in `web/`'s eslint/minimatch chain, needing `npm audit fix --force` major bumps. Outside `8fa7261b`'s REQ-01 and outside epic REQ-02, both of which scope to `--omit=dev`. Not a criterion violation; recorded so the final reconciliation sees it.
- `.github/workflows/security-audit.yml:49` runs `npm audit --production` — the deprecated alias, with no `--audit-level` — so CI enforces a laxer threshold than epic REQ-02 states. Belongs to `f7f80d53` ("Enforce reusable blocking dependency audits") in wave 2. Confirm it is closed there before the epic's gates run.

## Reference artefacts

- Epic: `jit issue show b80e3c70`
- Wave plan and per-issue status: `dev/active/b80e3c70-v1-release/progress.json`
- Dispatch snapshot for the leak check: `/tmp/lead-pre-dispatch-20260729-141201.txt`
- Skill references: `references/lead-review-protocol.md`, `references/worktree-dispatch-protocol.md`, `references/escalation-policy.md`
- Design docs: none linked to the epic (`jit doc list b80e3c70` is empty)
