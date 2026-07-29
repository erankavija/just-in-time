# Handoff — Version 1.0 release (b80e3c70) — session 1

**Date:** 2026-07-29
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `b80e3c70` — state: backlog, assigned `agent:jit-execution-lead`
- Wave in progress: wave 1, sub-wave 1a of 7 waves
- Children summary: 0 done, 8 claimed and dispatched, 14 backlog/ready, 0 rejected
- Active claims: `7edd2fe8`, `8fa7261b`, `bb302dbd`, `f04f7888`, `f289ff18`, `c7f8ebc7`, `eeee8a1a`, `985ab96d` — all `agent:worker`, claimed 2026-07-29 ~14:12Z
- Open escalations: None.
- Progress file: `progress.json` in this directory

Nothing is merged to `main`. No gate has been evaluated. No issue has been
transitioned. Every worker branch is anchored to `a8182e6f`.

Worker branch state at handoff (`worktree-agent-<short-id>`, worktrees under
`.agents/worktrees/agent-<short-id>`):

| Issue | HEAD | Commits | Uncommitted | Reported |
|---|---|---|---|---|
| 7edd2fe8 | 0c6baa2d | 3 | 4 | no — still working |
| 8fa7261b | 45bd85c5 | 2 | 0 | yes, in full |
| bb302dbd | 0d9d28ba | 3 | 0 | yes; one lead question outstanding |
| f04f7888 | a8182e6f | 0 | 10 | no — still working |
| f289ff18 | f08ae5f9 | 1 | 3 | no — still working |
| c7f8ebc7 | c8750579 | 1 | 1 | no — still working |
| eeee8a1a | ac46b11d | 2 | 0 | yes, in full |
| 985ab96d | 8d538c93 | 1 | 0 | no — still working |

## What just happened

- Discovered the epic already broken down: 22 children, `jit validate --scope b80e3c70` exits 0, so Section 3 was skipped for Section 4. No planning/breakdown bracket node exists (`jit issue list --label brackets:b80e3c70` returns 0); the decomposition arrived with commit `284371ce`, which replaced rejected epic `8b05a612`.
- Computed 7 topological waves over the 22 children. Wave 1 holds 11 zero-dependency issues; split into 1a (8), 1b (2), 1c (1) on file-footprint collisions.
- Assigned the epic, claimed the eight 1a issues, wrote `progress.json`, committed as `a8182e6f`.
- Created 8 worktrees via the dispatch script, all SHA-verified against `main` HEAD `a8182e6f`.
- Dispatched 8 `general-purpose` workers, each barred from `.jit/` writes, state transitions, and gate evaluation.
- `eeee8a1a` — returned; replaced the `resolve_footprint()` config lookup with the literal `FOOTPRINT=("docs")`. Sent back for a stale-narrative sweep it had declined; it then fixed 5 stale line-range citations in `dev/active/e204e63d-derived-package-sources/e204e63d-investigation.md` and answered the single-source-prose challenge (no config field denotes the adopter surface; `citation_scan_roots` is a whole-repo scan list including `crates/`, and deriving the default from the gate's own override is circular). Answer accepted.
- `8fa7261b` — returned; mermaid 11.12.2→11.16.0, axios 1.13.2→1.18.1, both minor, zero consumer migration, `found 0 vulnerabilities`, no override/`.npmrc`/audit-level escape. Added 4 tests driving the real mermaid module (previously fully stubbed).
- `bb302dbd` — returned; zod 3→4 major plus SDK 1.0.4→1.30.0. Evidenced the advertised protocol surface is byte-identical across both dependency sets (sha256-equal 971,821-byte capture over 112 tools, 37 outputSchemas, and the initialize handshake), and that exactly one assertion in its final suite is version-dependent.
- Lead accepted the zod 4 major (see Decisions below) and asked one outstanding verification question (see What to do next).

## What to do next

- [x] **Closed.** `bb302dbd`'s default-containment question is answered and verified — do not re-open it. Exactly one advertised default exists across all 112 tools at both dependency sets and all three surfaces (`jit_item_search.query = ""`, optional), so the non-empty-default case that would change CLI argv under zod 4 is empty. A differential sweep over all 112 tools shows 0 whose `(accepted, argv)` differ; the sole delta is `jit_item_search`'s intermediate `resolvedArgs` going `{}` → `{"query": ""}`, absorbed by `buildCliArgs` dropping empty-string positionals (`cli-executor.js:105`). Structural bound: `tool-generator.js` propagates defaults only for positional args (`:48-50`), with no `flag.default` branch in the flag loop (`:59-77`), so a flag default could never reach `validateArguments` at all. `bb302dbd` is ready to merge and review on its evidence.
- [ ] Collect the remaining 5 reports: `7edd2fe8`, `f04f7888`, `f289ff18`, `c7f8ebc7`, `985ab96d`. If a worker pings idle without reporting, ask it — see trap 4.
- [ ] Run `.agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh` before any commit on `main`. The dispatch snapshot is `/tmp/lead-pre-dispatch-20260729-141201.txt`.
- [ ] Merge each branch into `main` sequentially with `git merge --no-ff`, running `scripts/verify-commit-builds.sh` after **each** merge before the next one.
- [ ] Evaluate gates **sequentially**, from the main checkout, with an explicit `cwd` — see trap 5.
- [ ] Review each issue through all six tiers of `references/lead-review-protocol.md` before transitioning anything to `done`.
- [ ] Fix the stale comment in `.jit/gates.toml` under `[gates.checker.env]` above `DOCS_FOOTPRINT = "docs/"`, which asserts the checker's fallback derives from `[documentation].permanent_paths`. `eeee8a1a` makes that false. It is a TOML comment, not the `description` field, so no gate contract changes and `docs/reference/rules-and-gates.md` stays projection-fresh. Commit under `jit:eeee8a1a`.
- [ ] Add `CHANGELOG.md` entries for the dependency majors. Workers were kept out of the root manifest, so this is the lead's.
- [ ] Then dispatch sub-wave 1b (`f9e42a43`, `ef118aea`), then 1c (`a122b9b3`) alone.
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
