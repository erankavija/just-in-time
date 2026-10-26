# Handoff — Give each test property one owning test on shared fixtures and contracts (c780514e) — session 2

**Date:** 2026-10-25 (Europe/Helsinki)
**Session number:** 2
**Prior handoffs:** `dev/active/c780514e-test-topology/handoff.md` (session 1)

## Current state

- Epic: `c780514e`. The tracker reports `backlog`, a container rollup state; the work itself is under way.
- Wave in progress: 2 of 10. Wave 1 closed (27/27).
- Children: 29 done, 13 in_progress, 29 ready, 77 backlog, 0 rejected (148 implementation entries in `progress.json`).
- Active claims are all held by `agent:jit-execution-lead`:
  - finite leases, acquired 2026-10-24T23:16–2026-10-25T00:55: 5353463c, 870e8e46, eaf543c7, fec9b43b, b5faab2f, 9c3b9da2, e7764cb1, 7bf48536, 5f4daf6b, e230e621, 8e69776c, c9442f7f, 37218dff;
  - stale indefinite lease on unrelated 3d9a0ce3, left untouched as in session 1.
  - Leases expire after 4–8 h. Reacquire before any gate evaluation.
- Open escalations (two, both recorded in `progress.json` `escalations`): JitCli capability gaps (870e8e46, 9c3b9da2, 8e69776c) and the eaf543c7 rework limit. See Open questions.
- Progress file: `dev/active/c780514e-test-topology/progress.json` (reviews, dispatch log, rework counts, escalations, pitfalls).
- Parallel session: 2195c98f lead (`just-in-time-63`) on main. Coordination is agreed:
  - same lock forms;
  - exact-path commits, and they preserve our `events.jsonl` tail;
  - their 2ac50581 edits `cli_issue/quiet_mode_tests.rs` (one token). Whoever lands second merges the other first.
  - They wait on our fixture tasks. Message them when fixture tasks land.

### In-flight work at handoff

- **Slot A, `.agents/worktrees/agent-22355a56`, branch `worktree-agent-w2-cli-issue`.**
  - Committed and unmerged: b5faab2f `bcbf44664`, e7764cb1 `66af637ba`, 7bf48536 `1a60e7e20`. These passed lead review and 339/339 cli_issue tests; their cargo-ci and code-review gates have not run.
  - The Codex gpt-6-luna follow-up for fec9b43b, 9c3b9da2 and 5f4daf6b **finished after this handoff was drafted**. Its edits are **uncommitted** in this worktree and not yet lead-reviewed:
    - fec9b43b: 6 files, 44 tests.
    - 5f4daf6b: 6 files, 50 tests. It also deleted the suite-root `TaxonomyRepo` copy from cli_issue `main.rs`, since 5f4daf6b moved the last caller. Commit that `main.rs` change with 5f4daf6b.
    - 9c3b9da2: nothing changed. Beyond the serve test, its failure-probe tests (`test_recorded_failure_arms_emit_the_canonical_error_envelope`, `test_failure_probe_coverage_matches_clap_arms_and_emits_typed_envelopes`) need a committed HEAD and a linked worktree. That is the same fixture gap as open question 1.
    - In the sandbox, 338/339 cli_issue tests pass; the one failure is the serve-port bind, a sandbox artefact. Re-run outside the sandbox.
    - Brief: `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/287df093-c878-4892-a6f1-7223cf12cdad/scratchpad/brief-b-cli-issue2.md`. Report: `.../scratchpad/codex-cli-issue2.txt`.
- **Slot B, `.agents/worktrees/agent-de880d9c`, branch `worktree-agent-w2-crw1`.** Committed and unmerged; lead review passed; cli_repo_workflow 406/406:
  - e230e621 `f147f4d11`
  - 8e69776c `13c038953` (partial)
  - c9442f7f `2b0462f1d`
  - 37218dff `adf684b75`
- **On main:**
  - 5353463c fix `da0df2152` addresses code-review run 6b170ef0's F1 and F2; cargo-ci and code-review re-runs are pending.
  - The cli_gate merge `5f0e15931` landed, and cargo-ci passed for 5353463c, eaf543c7 and 870e8e46.
- Main working tree holds uncommitted epic tracker files and `progress.json`. The failed code-review runs `6b170ef0` and `da226023` are committed with this handoff.
- `land-cli-issue-a` is a throwaway branch pointing at slot A's three reviewed commits. Delete it after landing.

## What just happened

- Resumed on the user's order. Reacquired the df505379 and fa9bf296 leases and merged main into both worktrees.
- fa9bf296: the lead rendered the projection and recorded evidence. Landed with the five other self-test gate tasks in merge `650a3badf`; every gate passed, including six code reviews with no findings. Closed.
- c11bcdaa: Codex Luna removed the actionlint SKIP branch. The lead made the gate run the script directly and updated `dev/workflow-contract.md`. Closed.
- 89406e14: gate inputs changed from `.jit/config.toml` to `profiles/jit-default`. Closed.
- 2c453337, 7495521e and 0951556f: gate inputs were already complete; only the planning-history sentence was dropped. Closed. Missing-tool probes are recorded in the commit bodies.
- df505379 (Claude Sonnet): typed list envelopes. The lead removed per-shape `count == len` re-assertions, which `ListEnvelope` owns. Merged `1a8ef7fe6`; cargo-ci, mcp-ci (merged binary first on PATH) and code-review passed. Closed; wave 1 closed.
- cli_gate batch (Claude Sonnet: 870e8e46, 5353463c, eaf543c7): two lead-review reworks for helpers that built repositories and duplicated seeding helpers, which now live in cli_gate `main.rs`. Merged `5f0e15931`.
  - Code review then failed 5353463c (F1 id read from `.jit/issues`; F2 restating rustdoc) and eaf543c7 (F1 same rustdoc). The lead fixed both on main in `da0df2152`; 141/141 cli_gate tests pass.
- cli_issue batch (Codex Luna, first run): b5faab2f, e7764cb1 and 7bf48536 done. The lead replaced Codex's per-file `assert_command` adapter, duplicated five times, with `cli.command(..)` plus `assert_cmd::prelude`. Committed per issue.
  - Codex skipped fec9b43b, 9c3b9da2 and 5f4daf6b. Stdin turned out to be no gap; the serve sibling-binary test is a real gap. Re-dispatched.
- cli_repo_workflow batch (Claude Sonnet): 4 issues committed. 8e69776c is partial: `divergence_recovery_journey_tests.rs` needs commits and linked checkouts.
- Deviation, recorded in `surfaced_pitfalls`: the cli_gate batch was dispatched before wave 1 closed. It was contained by not merging until wave 1 closed.

## What to do next

- [ ] Read the user's answers to the two open questions first.
- [ ] Slot A:
  - Check whether Codex finished (`codex-cli-issue2.txt`).
  - Review the uncommitted diff against `brief-b-cli-issue2.md`: no repository-building helpers, no `split_whitespace` id scraping, no duplicated adapters, shared `jit::test_taxonomy` for taxonomy callers.
  - Run `cargo nextest run -p jit --test cli_issue --no-fail-fast` outside the sandbox.
  - Commit per issue by file footprint (see the "This entry covers" line in each issue).
- [ ] Commit main's pending epic tracker changes, then land in this order with `scratchpad/land.sh` (run it from a clean main; it refuses non-tracker dirt, including `progress.json`):
  1. 5353463c on main (branch argument `main`), then `review.sh 5353463c`. This is its rework 2 of 2; a further FAIL escalates.
  2. Slot B: `land.sh worktree-agent-w2-crw1 "<msg>" e230e621 c9442f7f 37218dff 8e69776c`, then reviews for the first three. If 2195c98f's e1a6c8c2 has landed (config writes now resolve `VirtualPath::CONFIG` via `RepositoryLayout`), first merge main into the branch: e230e621 (`f147f4d11`) edits `cli_repo_workflow/project_config_tests.rs`, which e1a6c8c2 also changes. e1a6c8c2 also touches `test_utils.rs` (new `repository_layout_for_data_root` block), `test_harness.rs`, `fast_docs_templates/`, `fast_rules/` and `cli_item_validate/invariant_registry_story_tests.rs`; check these before dispatching 07f4b1d4, b93687b3 and 717f9c2f.
  3. Slot A: the three reviewed issues plus any accepted follow-up issues.
- [ ] Close passing issues with `scratchpad/close.sh "<msg>" <ids>`, then update `progress.json` (`prog.py status/review`).
- [ ] Remaining ready wave-2 work, dispatched to the two worktrees, alternating Claude and Codex (next dispatch: Claude):
  - spawn migrations: bac4423b, b93687b3, 07a1728d, 717f9c2f, 173d98d5 (owns cli_repo_workflow `main.rs`; do it last);
  - fixture-issue adoptions: 33be0c83, d9598a97, 2f95b4f6, b90b1660, 74b2a0d3, 9964af3f, 58dbb1a0, 19136324, 7995aa5c, b612cde1;
  - 5876e131, 07f4b1d4 (rewrites the profiled-fixture section of `test_utils.rs`; tell 2195c98f), 529f621d, f4deb00a, 8b6285a6, 7801f623, b511222f, 64f04da6, 1d52a376, 2f717d93, e21ca76d;
  - 53f78490 becomes ready now that df505379 is done.
  - Footprint overlaps are listed in `scratchpad/w2-foot.tsv`. Serialize these groups:
    - `commands/mod.rs`: 529f621d, 53f78490, 74b2a0d3, b511222f;
    - `commands/item.rs` and `commands/validate.rs`: 74b2a0d3/53f78490/e21ca76d and 9964af3f/07f4b1d4/e21ca76d;
    - storage files: b612cde1, 64f04da6;
    - `domain/queries.rs`: 19136324, 1d52a376;
    - cli_item_validate: 07f4b1d4 with b93687b3 and 717f9c2f.
- [ ] Generate briefs with `scratchpad/mkbrief.py` from `scratchpad/batch-template.md`. The template already carries the helper and self-check rules learned this session.
- [ ] Message just-in-time-63 when e7764cb1 (`quiet_mode_tests.rs`) and the fixture-issue adoption tasks land.

## Traps — do not repeat these

Session-1 traps (`handoff.md`) stay in force, especially: `CARGO_INCREMENTAL=0`, lock forms and the load/disk guard, no new worktrees, no blanket cache cleanup, and durable measurement evidence.

- **Do not dispatch the next wave before the current one closes.** This session sent the cli_gate batch while df505379 was open. Check that `current_wave` is done before dispatching.
- **"No repository setup helper" means literally none.** Session 2 needed three review rounds on cli_gate for `setup_*`, `preset_repository`, `bracket_repository` and `(JitCli, String)` builders. Each test constructs its own `JitCli`; helpers only seed a `&JitCli` they are given.
- **Workers duplicate small adapters per file.** Codex wrote `fn assert_command` five times, and Claude wrote `define_auto_gate` three times. Check this before landing; the reviewer and `@/inv/convention-convergence` flag it.
- **Restating rustdoc on test helpers fails code review** (runs 6b170ef0, da226023). Tell workers to add no doc comment to trivial helpers.
- **Stdin is not a JitCli gap.** Use `assert_cmd::Command::from_std(cli.command(..)).write_stdin(..)` inline. The first Codex run wrongly stopped three issues on it.
- **Codex sandbox blocks TCP binds.** Serve tests fail there with exit 5. Verify such failures outside the sandbox before believing them.
- **Codex cannot commit.** The lead commits per issue footprint and must keep an issue that owns a target `main.rs` last.
- **Idle Claude workers are not woken by their own background shells.** w-crw1 idled waiting on clippy. Tell workers to run cargo in the foreground with a long timeout.
- **`land.sh` refuses any non-tracker dirt on main, including `progress.json`.** Commit progress before landing. A run that printed "main dirty" or "dirty after merge" did nothing, and two such failures happened this session. The parallel session's merges can also leave transient dirt; retry.
- **Do not run `review.sh` unconditionally after `land.sh`.** Gate on the cargo-ci pass line, as the slot-A command did. Reviewing unmerged commits wastes a review.
- **Lease limit: at most two indefinite leases per agent.** The stale 3d9a0ce3 lease occupies one slot. Use finite TTLs.
- **Wave-2 issues share files** (see the overlap groups above). Never put overlapping issues in the two worktrees at once.

## Open questions needing invoker input

- **Question:** How should the shared CLI fixture cover the tests it cannot serve today?
  - **Context:** `crates/jit/src/test_cli.rs` `JitCli` cannot (1) stage and commit, (2) spawn an alternate program under its environment scrub, or (3) offer a linked-checkout setup. This blocks:
    - 870e8e46: `cli_gate/gate_evaluate_all_test.rs` (8 tests) and `gate_evaluate_rerun_test.rs` (2 tests);
    - 9c3b9da2: `cli_issue/command_exit_code_projection_tests.rs::test_command_exit_codes_serve_foreground_passthrough_is_dual_form`;
    - 8e69776c: `cli_repo_workflow/divergence_recovery_journey_tests.rs`.
    - Later git-fixture tasks will likely hit (1) too.
  - **Options:**
    - A: one new leaf task in this epic for these fixture capabilities, each with one owning test in `cli_process_fixture_tests.rs`, optionally adding `JitCli::with_named_root`; the three issues depend on it.
    - B: named local exceptions under `@/inv/convention-convergence`, each with a tracked convergence condition.
  - **Recommendation:** A. It keeps one fixture form, and later tasks need commit support anyway.
- **Question:** May eaf543c7's code-review gate be re-run on current main?
  - **Context:** It used both reworks (two lead-review rounds on repository-building helpers). Code-review run da226023 then failed only on restating rustdoc in cli_gate `main.rs`. That rustdoc is removed in `da0df2152`, which is attributed to 5353463c because the same finding was reported against it.
  - **Options:** A: authorize the re-run. B: give guidance or reject the issue.
  - **Recommendation:** A. The cited lines are already fixed.

## Reference artefacts

- Epic: `jit issue show c780514e`; plan `dev/active/c780514e-test-topology/plan.md`; manifest `breakdown.json`; progress `progress.json`; investigation `investigation.md`; prior handoff `handoff.md`.
- Measurements: `dev/benchmarks/test-topology-c780514e/README.md`. Gate evidence: `.jit/gate-runs/*/result.json`.
- Shared fixtures: `crates/jit/src/test_cli.rs`, `crates/jit/src/test_taxonomy.rs`, `crates/jit/src/test_utils.rs`, `crates/jit/tests/cli_repo_workflow/cli_process_fixture_tests.rs`, `dev/TESTING.md`.
- Session-2 lead tooling in `/tmp/claude-1000/-home-vkaskivuo-Projects-just-in-time/287df093-c878-4892-a6f1-7223cf12cdad/scratchpad/`. This is a convenience copy that is lost on reboot; regenerate from this handoff if it is absent.
  - `land.sh <branch> <subject> <ids>`: merge plus cargo-ci under the `-x` window with the load guard.
  - `review.sh <ids>`: parallel code reviews plus findings.
  - `close.sh <msg> <ids>`: verify gates, mark done, release leases, exact-path commit.
  - `prog.py`: progress edits.
  - `mkbrief.py` and `batch-template.md`: batch briefs.
  - Briefs `brief-b-*.md`.
- Skills: `~/.claude/skills/jit-execution-lead/SKILL.md`, `codex-subagents`, `.agents/skills/jit-parallel`, `.agents/skills/jit-manage`.
