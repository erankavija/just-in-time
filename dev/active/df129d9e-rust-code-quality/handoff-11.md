# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 11

**Date:** 2026-11-04
**Session number:** 11
**Prior handoffs:** [handoff.md](handoff.md), [handoff-2.md](handoff-2.md) through [handoff-10.md](handoff-10.md)

Outgoing lead: session c12148f1 (`agent:df129d9e-lead`). Incoming lead: just-in-time-7c. Steward: just-in-time-6a (the user's channel; every scope change and ruling goes through 6a).

## Current state

- Epic: `df129d9e` — state: backlog (children carry the work; the epic is not claimed).
- Wave in progress: wave 7 of 13 (handler migrations onto the library handler contract). Waves 1–6 are done.
- Children summary (label `epic:rust-code-quality`): 54 done, 14 in_progress, 1 ready (4a01ccba), 19 backlog. Counted after c9103fcf closed; recount with `jit query all --label epic:rust-code-quality --json`.
- Wave 7 done this session: b23faa59 (pathfinder), 1eb949a5 (per-test measurement), 12c0f3fb (handler contract covers every arm shape), e5d618ee (server statuses from classification), 8a14e8e9 (ENOSPC is a host fault in cargo-ci), 153c9851 (config), 9529b9ac (warnings in every JSON response), c9103fcf (doc). Standalone, outside the epic: 26307dfd (mcp-ci tests the built binary) done; 8c67b21a ready, not started.
- Active claims: every wave 7 in_progress issue is assigned to `agent:df129d9e-lead` with a 24 h lease (`jit claim acquire <id> --ttl 86400`). Release or take them over as below.
- Open escalations: none awaiting the user. All open points below have steward rulings.
- Progress file: [progress.json](progress.json) (rework counts, per-issue vendor and notes, pitfalls, line target).

### Wave 7 issue table

| Issue | What | State | Branch head | Worktree | Next step |
|---|---|---|---|---|---|
| f930acf2 | version | in batch | `worktree-agent-f930acf2` | yes | land with batch |
| 8f297e60 | init, apply | in batch | `worktree-agent-8f297e60` | yes | land with batch |
| b25c2e54 | events | in batch | `worktree-agent-b25c2e54` | yes | land with batch |
| 1a4f2ebd | item | in batch | `worktree-agent-1a4f2ebd` | yes | land with batch |
| 8c46fc85 | serve | in batch | `worktree-agent-8c46fc85` | yes | land with batch |
| 66790c84 | claim, recover | in batch | `worktree-agent-66790c84` | yes | land with batch; tell 3e |
| 98cc2097 | search | in batch | `worktree-agent-98cc2097` | yes | land with batch (rework 2 of 2 used) |
| 3cf69ecf | archive | in batch | `worktree-agent-3cf69ecf` | yes | land with batch |
| 0fe46027 | dep | in batch | `worktree-agent-0fe46027` | yes | land with batch |
| 71f5296c | hooks | in batch | `worktree-agent-71f5296c` | yes | land with batch |
| 8e91126b | project | in batch | `worktree-agent-8e91126b` | yes | land with batch |
| 52110fb4 | label | in batch | `worktree-agent-52110fb4` | yes | land with batch |
| 06ee9b67 | validate | in batch | `worktree-agent-06ee9b67` | yes | land with batch; tell a4 |
| 20fc3b92 | graph | in batch | `worktree-agent-20fc3b92` | yes | land with batch |
| 10751360 | issue reads | dropped from batch | `845d1fe51` | yes | `E0119`: both it (`output/issue.rs:176`) and 1a4f2ebd (`output/item.rs:22`) implement `HumanRender for ItemShowResult`; route a merge-and-keep-one rework (count 1 of 2); tell 3e |
| 1ac651c7 | gate definitions, presets | Codex done, unreviewed | `ac9220229` (lead-preserve) | yes | review, CI, land; tell 3e and a4 |
| cd8e04cc | worktree | Codex done, unreviewed | `2cb30f0ac` (lead-preserve) | yes | review, CI, land |
| 182f4c03 | snapshot, migrate | Codex partial | `e1bff4aae` (lead-preserve) | yes | review; +68 lines to reconcile |
| cc4ea8a9 | profile | Codex partial | `b7827c17a` (lead-preserve) | yes | merge main once 8f297e60 is on main, then Codex resume |

The batch is the staging branch `worktree-agent-w7batch` (worktree `/mnt/nvme/jit/agents/worktrees/agent-w7batch`), HEAD `5376c344f`: one `merge(jit:<id>)` commit per issue on top of main 847c50458, nothing rewritten (plan D-48). Merge SHAs: f930acf2 8f6e9ffd8, 8f297e60 07459d8d8, b25c2e54 35f81d74d, 1a4f2ebd 1bea4f8ee, 8c46fc85 0812f4bc7, 66790c84 756036fa4, 98cc2097 2c8cc3963, 3cf69ecf 9feb532e6, 0fe46027 71aa20c1b, 71f5296c 17401219a, 8e91126b 0102810ac, 52110fb4 0e2f3e87d, 06ee9b67 be080d3d6, 20fc3b92 5376c344f.

Checks at `5376c344f` (worker TMPDIR, advisory): fmt clean; clippy zero warnings; `cargo test -p jit --lib` 3035 passed; jit-server tests pass; doctests 25 pass; every CLI target passes except `cli_issue`, 3 failures:
- `command_exit_code_projection_tests::test_command_exit_codes_every_row_is_verified`: projected rows differ from bound rows (row not yet identified).
- `failure_probe_coverage_tests::test_failure_probe_coverage_matches_clap_arms_and_emits_typed_envelopes`: `hooks install` renders `GENERIC_ERROR` (from 71f5296c's handler).
- `list_envelope_tests::test_list_envelope_count_equals_collection_length_for_every_list_emitting_command`: "list-emitting commands without an invocation: [rdeps]" (from 20fc3b92's top-level `rdeps` registration).

Blocked, not landable yet:
- The final `git merge main` conflicts in `crates/jit/src/main.rs`: main's 62b8fa62d (jit:614a6f97, 3e) changed the `claim renew`, `heartbeat` and `force-evict` arms to take an issue id instead of a lease id and the claim list to show `lease.issue_id.short_id()`, while 66790c84 moved those arms into `dispatch/claim.rs`. That change must be ported into the claim handler: route it to a worker as a 66790c84 follow-up (its rework count is 0 in progress.json; check before dispatching).
- The 3 `cli_issue` failures above: route each to its issue's worker as a follow-up on its own branch (hooks to 71f5296c, rdeps to 20fc3b92; the projection row: find the row first). Re-merge the fixed branches into the batch, or rebuild the batch from current main.
- The integrator resolved a few semantic overlaps beyond additive lines: removed unused imports (`VirtualPath`, `resolve_issue_filter`, `anyhow`), `data_root: jit_dir` → `data_root: _` in the serve path, re-pointed `test_resolve_output_schema_unregistered_leaf_takes_legacy_table_schema` from `version` to `status`, moved exit-code rows to 8c46fc85's non-`Option` form and set the three `validate` rows to 4 (06ee9b67), and regenerated `docs/reference/error-codes.md`. Review these in the merge commits before landing.

## What just happened

- Wave 7 fan-out: 22 issues dispatched (13 Claude: Sonnet and Haiku; 9 Codex: gpt-6.1-sol, gpt-6-luna, gpt-6-astra). Briefs were generated by `lead-tools/` scripts (see Reference artefacts).
- Shared handler form gaps surfaced by workers were landed once, at the source: 12c0f3fb added the executor-free, single-leaf, concrete-store, transitional pass-through (split groups) and union-schema forms, `render_human(&self, mode)`, and the `Outcome` constructors plus shared resolvers (plan D-38 to D-41). 9529b9ac added a success or findings outcome's `warnings` to every JSON response and schema (D-42, D-43, D-49).
- Steward rulings recorded as `## Decisions` items on: f930acf2 (later moved to 12c0f3fb), 8c46fc85 D-01 and D-02, c9103fcf D-01 and D-02, 10751360 D-01, 1ac651c7 D-01, 20fc3b92 D-01, 9529b9ac D-01, 5687e281 D-01 and REQ-08 (removes the transitional pass-through form). Criteria amended with steward approval: cc4ea8a9 REQ-01, REQ-03 and REQ-04; cd8e04cc REQ-02 and REQ-06; c9103fcf REQ-07.
- New tasks filed (steward requests): 8c67b21a (dispatch guard ignores the data root; standalone v1.0), 26307dfd (mcp-ci builds the binary; done), 12c0f3fb, 9529b9ac, 8a14e8e9 (all done).
- Host faults handled: /tmp tmpfs full and later out of inodes (authoritative runs now use /dev/shm, D-45 and the steward's /dev/shm rule), cargo-pool-run token waits inside nextest's timer (TIMEOUTs; steward has it with the user; D-43 holds f487aeef), the flaky drift test (3e fixed it at 76a60eb2f), the sccache daemon whose TMPDIR was deleted (steward restarted it at a stable TMPDIR).
- Wave 7 landings collided on additive registration lines (`pub mod`, `REGISTERED_GROUPS`, `main.rs` arms). The steward approved one batch integration (D-48).
- The integrator merged 14 branches into `worktree-agent-w7batch` and dropped 10751360 (`E0119`). It was mid-run when the user's 5-hour limit hit and resumed after the reset. Its final main merge conflicts with 614a6f97 and three `cli_issue` tests fail, so the batch is not landed.
- Wind-down per the steward: no new dispatch. Codex results preserved on their branches as `wip(jit:<id>): lead-preserve …` commits, not merged.

## What to do next

- [ ] Read this handoff, the plan's decisions D-29 to D-49 ([plan.md](plan.md)), and [progress.json](progress.json). Message 6a when you take over; 6a releases nothing to you without that.
- [ ] Copy `lead-tools/land-one.sh` and `lead-tools/close-issue.sh` into your own scratchpad before running them (they keep their logs and lock beside themselves; running them from the repository leaves untracked files on main). Set `R`, `W` and the agent id at the top to yours, or keep `agent:df129d9e-lead` if you take over the claims.
- [ ] Unblock the batch (see the blocked list under Current state): port 614a6f97's claim-arm change into 66790c84's handler, fix the three `cli_issue` failures on their issues' branches, then merge main into `worktree-agent-w7batch` (or rebuild it from current main by re-merging the 14 branches). Then run one authoritative `./scripts/cargo-ci.sh` in `agent-w7batch` (under the full-CI lock with `TMPDIR=/dev/shm/jit-ci/<you>` and the sccache guard), land the branch on main with `jit worktree exec -- bash .agents/skills/jit-execution-lead/scripts/land.sh worktree-agent-w7batch` (or `land-one.sh w7batch` with `SKIP_GATES` unset and no gate loop, since w7batch is not an issue), then evaluate each of the 14 issues' gates and close each with `close-issue.sh <id>`. If staging CI fails, bisect by merge commit before landing anything (D-48). Each close: reclaim the worktree with `.agents/skills/jit-execution-lead/scripts/reclaim-worker-worktree.sh <id>`.
- [ ] 10751360: after the batch lands, route a merge-main rework to a worker that keeps one `HumanRender for ItemShowResult` (rework 2 of 2), then land singly. Its branch has the code-review fixes for schema names (`IssueListResponse`, `IssueSearchResponse`) and the multi-id error context.
- [ ] 1ac651c7 and cd8e04cc: lead-review the Codex diffs (`git diff main...worktree-agent-<id>`), check no `.jit` file changed, merge main into them yourself (Codex must not run writing git commands), land singly.
- [ ] 182f4c03: lead-review; its +68 lines needs reconciling before acceptance (the user's goal is net reduction; see REQ-10 below). Route any trim to a worker.
- [ ] cc4ea8a9: after 8f297e60 is on main, merge main into its branch yourself and resume Codex with `lead-tools/resume-12c0f3fb.txt`, the Codex git rule and the worker TMPDIR rule appended. Its criteria were amended (REQ-01, REQ-03, REQ-04); 8f297e60 moved the profile parsers and made an unreadable `--values-file` an `InvalidArgumentError`, so `profile apply` needs a row and a dual-form binding for exit 2.
- [ ] After wave 7: wave 8. 4a01ccba (refactor slow tests) is ready: its dispatch note in progress.json says to measure every listed test on a disk-backed TMPDIR as well as /tmp (D-45). Do NOT let f487aeef tighten per-test limits until the steward reports the cargo-pool-run token-wait question settled (D-43). fba8d478 follows f487aeef.
- [ ] Standalone 8c67b21a (dispatch guard; gates worktree-cache-test, code-review, cargo-ci): adds a data-root field to `jit worktree info --json` and makes `dispatch-worker-worktree.sh` tolerate uncommitted data-root changes. Once it lands, delete `lead-tools/dispatch-tracker-tolerant.sh` (the private copy) and use the skill script.
- [ ] Before wave 8 dispatch, recheck 0bc448fb against main: a4 is removing most validation `capture_*` helpers.

## Traps — do not repeat these

- **Do NOT run a landing lock as an inherited fd.** `exec 9>lock; flock 9` was inherited by a daemonized sccache and deadlocked two landings for two hours. Take serial locks with `flock -o` (land-one re-execs itself under `flock -o`).
- **Do NOT set TMPDIR to /mnt/nvme for authoritative runs.** a4 reproduced: on ext4 the drift test and four sync-heavy tests fail (directory order, fsync cost). Authoritative runs use `/dev/shm/jit-ci/<lead>`; workers' advisory runs use `/mnt/nvme/jit/tmp/<lead>-workers` and report the known disk-only failures without reworking (steward rule).
- **Do NOT let an authoritative run start the shared sccache daemon.** A daemon started under a per-run TMPDIR died when that run cleaned its directory, breaking every compile on the host. land-one's `sccache_guard` uses sccache only if a daemon already runs; otherwise `CARGO_CI_NO_SCCACHE=1`. If the daemon is missing, tell 6a; 6a restarts it with `TMPDIR=/dev/shm/sccache-tmp`.
- **Do NOT let Codex run writing git commands.** Its sandbox makes git metadata read-only; "merge main" produced half-merges and out-of-band merges that copied main's uncommitted `.jit` lines into the branch. The lead merges main into Codex branches; the Codex brief appendix `lead-tools/codex-git-rule.md` forbids it. If it happens anyway, record a proper two-parent merge (`git commit-tree … -p HEAD -p <main commit>`) and restore `.jit` files to the merged parent before landing.
- **Do NOT give Codex a TMPDIR outside its sandbox.** Codex runs reported `ReadOnlyFilesystem` for `/mnt/nvme/jit/tmp/...` and needed escalated retries. Point Codex test runs at a TMPDIR inside its worktree's target dir or accept the retry; do not put it inside the git tree itself (archive tests fail when temp dirs sit inside a git repo).
- **Do NOT let a worker add a local variant of a shared form.** Several workers added their own `command_group!` rules, quiet hooks or pass-through clauses before 12c0f3fb; each had to be reverted. A gap in the handler contract goes to the steward and lands once at the source.
- **Do NOT cite a precedent that is only on a branch.** I told 98cc2097 to copy `Serve(ServeArgs)` "on main" when it was only on a branch; the worker was right to stop. Check `git merge-base --is-ancestor` before citing.
- **Do NOT retry a content failure.** land-one retries a host fault (exit 75, TIMEOUT-only, ENOSPC) at most once per landing (D-46). `--force` on a gate only when the reused run failed for a host fault and the code and binary are the right ones; name the reused run id in the plan (D-47).
- **Do NOT `cd` into a worker's worktree in your own shell.** The harness broadcasts the lead's working-directory change to workers as an "environment update"; one worker briefly believed its worktree was another's. Use `git -C <path>` and absolute paths.
- **Do NOT stop a landing past `land.sh`.** Check where a land-one run is (`ps`, its output) before stopping it; before `land.sh` it is safe, after it the merge is on main and only the gates remain.

Traps from handoff-10 and earlier remain in force unless resolved there.

## Open questions needing invoker input

None awaiting the user directly. Pending with the user via the steward:
- Question: should `cargo-pool-run` stop waiting for a jobserver token inside nextest's per-test timer?
  - Context: millisecond tests TIMEOUT at 20 s under parallel load; f487aeef's per-test limits would turn those into false failures.
  - Options: the runner takes tokens up front and sets test-threads; or leave it.
  - Recommendation: fix the runner before f487aeef tightens limits (D-43).

## Reference artefacts

- Epic: `jit issue show df129d9e`
- Plan and decisions: [plan.md](plan.md) (D-29 to D-49 are this session's)
- Progress: [progress.json](progress.json)
- Investigation: [investigation.md](investigation.md); line baseline: [line-count-baseline.md](line-count-baseline.md)
- REQ-10 tracking: production lines of `main.rs` + `dispatch/` + `output` must end below 8,700. 9,248 on main after c9103fcf (from 9,111 at b5bd434e8; 12c0f3fb added +233 for the forms). Measure with `python3 lead-tools/req10.py <rev>` or `python3 scripts/production-lines.py <rev>`. Most wave 7 handler moves add lines (+5 to +70 each); the planned deletions that must bring it below 8,700 are the legacy schema table (da30b92a), main.rs consolidation (5687e281) and OutputContext. Report per-merge deltas and whole-crate net to the steward.
- Lead tooling (copy to your scratchpad before use):
  - `lead-tools/land-one.sh <id>`: serial landing for one issue. It re-execs under `flock -o` on `land2.lock` beside itself (`LAND_LOCKED=1` skips that for a priority landing); waits while a `hold` file exists beside it; merges main into the branch (exit 3 on conflict); runs branch `./scripts/cargo-ci.sh` under `flock -o /tmp/jit-ci-turnstile.lock flock -o -x /tmp/jit-full-ci.lock` with `TMPDIR=/dev/shm/jit-ci/df129d9e` (emptied after each run) and the sccache guard; retries a host fault once (exit 75, TIMEOUT-only failures, ENOSPC); lands with `land.sh` through `jit worktree exec`; checks ancestry; evaluates the issue's gates (cargo-ci first, code-review last; `SKIP_GATES="<gate> …"` defers gates a ruling defers); prints `<id>: LANDED AND GATED`. Exit codes: 3 conflict, 4 dirty worktree, 5 branch CI failed, 6 not an ancestor, 7 a gate failed.
  - `lead-tools/close-issue.sh <id>`: verifies gates and completes one issue, committing the tracker change.
  - `lead-tools/dispatch-tracker-tolerant.sh <ids>`: private copy of the skill's dispatch script that ignores uncommitted `.jit/` changes; retired by 8c67b21a. Run as `LEAD_SNAPSHOT_DIR=<scratch> bash dispatch-tracker-tolerant.sh <ids>`.
  - `lead-tools/resume-12c0f3fb.txt`, `codex-git-rule.md`, `worker-tmpdir-rule.md`: brief appendices for resumed workers, Codex workers and worker test runs.
  - `lead-tools/codex-models.txt`: the Codex model and effort used per issue.
  - Codex invocation: `codex exec --ephemeral -s workspace-write -C <worktree> -m <model> [-c model_reasoning_effort=…] -o <out.txt> - < <brief>`; Codex cannot commit, so the lead reviews and commits with "Worker: Codex. Committed by the lead after review."
- Tracker state: this repository's tracker is format 3; never run `./scripts/install-jit.sh` until the steward's format 4 window (a fresh stop request comes first). The installed `jit` is at `/home/vkaskivuo/.cargo/bin/jit`.
- Coordination: just-in-time-3e (storage epic f8a6b298; overlaps claim, graph, server, gate runs), just-in-time-a4 (validation epic 9876a23a; overlaps validate and 0bc448fb). Rule: whoever lands second merges main and adapts. Tell each when an overlapping issue lands (66790c84, 1ac651c7, 10751360 to 3e; 06ee9b67, 1ac651c7 to a4).
