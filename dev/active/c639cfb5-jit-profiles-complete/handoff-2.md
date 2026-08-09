# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 2

**Date:** 2026-08-09T11:47:35+03:00
**Session number:** 2
**Prior handoffs:** `dev/active/c639cfb5-jit-profiles-complete/handoff.md`

## Current state

- Epic: `c639cfb5` — state: backlog, assigned to `agent:jit-execution-lead`
- Wave in progress: wave 2 of 18
- Children summary: 1 done, 1 in_progress, 22 backlog, 0 ready, 0 rejected
- Active claims: none for this epic. `eac6ec13` remains assigned to `agent:worker-eac6ec13`, but no live lease exists for it. Three unrelated indefinite leases are present for `bcff56be`, `c533ac18`, and `070ca05f`.
- Open escalations: none awaiting invoker input. The invoker authorized the targeted Wave 2 retry, then instructed the lead to hand off at its next review failure.
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir c639cfb5 dev/active` (reflects the above)

## What just happened

- Resolved the prior handoff's out-of-epic blocker: `45cd8529` was completed and integrated before epic delivery resumed.
- Completed Wave 1 issue `f2ef652a`; product commits `ddc01e62` and `c89f6932` landed on `main` through merge `810a10c7`, and the issue completion state landed separately in `d2b3eb6c`.
- Dispatched Wave 2 issue `eac6ec13` through Luna at xhigh in `.agents/worktrees/agent-eac6ec13`; initial implementation `6f9da1b1`, inventory repair `2b550db9`, and merge `9a4a5f00` established repeatable `id:`/`path:` selectors.
- Post-main review at `9a4a5f00` failed because profiled init collapsed repeated root occurrences and `profile show` rejected a second selector. Repair `0f964f23` closed both findings with occurrence-preserving results and tests.
- Review at `0f964f23` failed because `profile apply --dry-run` still rejected a second selector. The standard two-attempt limit was reached and escalated.
- Recorded the invoker's authorization for one targeted retry in `822ceccc` on `main`, reset the Wave 2 rework counter, and dispatched the well-specified repair through Luna at xhigh.
- Product commit `46fc9427` made dry-run count-wrapped and occurrence-ordered, updated the CLI/schema/MCP/docs contract, and added mixed path/id duplicate-order and non-mutation tests.
- Refreshed gates at product commit `46fc9427`: `cargo-ci` passed in an isolated clean target and `mcp-ci` passed against the provenance-pinned binary. The first cargo invocation failed only its incremental-cache preflight; its isolated rerun passed all workspace checks.
- Audited every prior review finding and confirmed the init, show, and dry-run findings have direct current implementation and regression-test coverage.
- The Tier 2.5 stale-consumer sweep found a new blocking defect at `.github/workflows/release-artifacts.yml:293`: the release smoke workflow still asserts `plan["status"]`, but dry-run now returns `plan["profiles"][0]["status"]` in a count-wrapped result.
- The configured `code-review` invocation then failed at run `d4a9a6fd-3900-4091-95de-2168e044fa0a` because the narrowed PATH omitted `/home/vkaskivuo/.local/bin/codex`; it produced no model verdict. Per the invoker's explicit instruction, stopped without another repair and wrote this handoff.
- Committed the authorized retry's gate evidence separately as `d3af63f6` on `worktree-agent-eac6ec13`. The worktree is clean; product commit remains `46fc9427` immediately below the evidence commit.
- Preserved two uncommitted main-worktree edits owned by the other in-flight agent: `docs/reference/cli-command-grammar.md` and `docs/reference/profiles.md`. No Wave 2 merge was attempted after the retry.

## What to do next

- [ ] Resume Wave 2 in `.agents/worktrees/agent-eac6ec13` on branch `worktree-agent-eac6ec13`; begin from clean HEAD `d3af63f6` and treat `46fc9427` as the reviewed product commit.
- [ ] Repair the stale release smoke assertion at `.github/workflows/release-artifacts.yml:293` for the count-wrapped dry-run result and add or update the cheapest test that executes that consumer contract.
- [ ] Repeat the stale-consumer sweep for the removed top-level `ProfilePlanResult` shape, including `.github/`, scripts, examples, docs, MCP, and tests; distinguish historical issue/audit prose from active executable consumers.
- [ ] Run `cargo-ci` and `mcp-ci` from a newly installed clean-provenance binary for the repair commit. Use a clean Cargo target or remove only the exact disposable incremental cache before `cargo-ci`.
- [ ] Run `code-review` with a PATH containing both the newly installed `jit` and `/home/vkaskivuo/.local/bin/codex`; do not count run `d4a9a6fd` as a substantive model review.
- [ ] Complete the six-tier lead review, including a resolution table for all four substantive findings: repeated init roots, repeatable show, repeatable dry-run, and the release smoke consumer.
- [ ] Before integration, coordinate the two existing dirty main documentation edits. The Wave 2 branch also edits `docs/reference/profiles.md`; preserve the other agent's bytes and do not stage or overwrite them.
- [ ] After branch and exact-main gates pass, merge Wave 2 dependency-ordered into `main`, complete `eac6ec13`, advance `progress.json` to wave 3, and dispatch `474a90a8`.

## Traps — do not repeat these

- **Do not treat the latest review run as a code finding.** Run `d4a9a6fd` lasted 141 ms and says `codex: command not found`; the substantive blocker came from the lead's Tier 2.5 sweep at `.github/workflows/release-artifacts.yml:293`. Include `/home/vkaskivuo/.local/bin` when pinning PATH for the next review.
- **Do not stop the stale-shape audit at Rust, docs, and MCP.** The active release workflow is an executable consumer of profile dry-run JSON and retained the superseded top-level `status` access. Sweep `.github/` and scripts whenever an external response shape changes.
- **Do not run `cargo-ci` against a target containing interactive incremental state.** Run `9f012727` failed only because `target/debug/incremental` and `target/jit-stale-child-test-cache/debug/incremental` were non-empty. The clean isolated rerun passed; clean up only the exact disposable target afterward.
- **Do not merge over the main worktree's documentation edits.** Another agent has uncommitted changes in `docs/reference/cli-command-grammar.md` and `docs/reference/profiles.md`, and Wave 2 changes the latter. Inspect and preserve both authors' intent before integration.
- **Do not restart the pre-authorization escalation debate.** The invoker explicitly authorized the targeted dry-run retry in main commit `822ceccc`; `progress.json` now counts the resulting failed lead review as attempt 1 after that reset. The current stop is the requested session handoff, not a request to reject the prior authorization.
- The graph-shape and direct-main traps in the prior `handoff.md` remain in force. Its dirty-main blocker involving `45cd8529` is resolved; do not re-open it.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Wave 2 issue: `jit issue show eac6ec13`; gates: `jit gate status eac6ec13 --all --json`
- Wave 2 worktree: `.agents/worktrees/agent-eac6ec13`, branch `worktree-agent-eac6ec13`, evidence HEAD `d3af63f6`, product commit `46fc9427`
- Blocking consumer: `.github/workflows/release-artifacts.yml:290`
- Design docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/breakdown.json`
- Research: `dev/active/c639cfb5-jit-profiles-complete/investigation.md`, `dev/active/c639cfb5-jit-profiles-complete/c639cfb5-research.md`
- Progress: `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Prior handoff: `dev/active/c639cfb5-jit-profiles-complete/handoff.md`
- External references: None.
