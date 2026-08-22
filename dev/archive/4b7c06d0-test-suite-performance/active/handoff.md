# Handoff — The workspace test suite runs in under 30 seconds (4b7c06d0) — session 1

**Date:** 2026-08-11
**Session number:** 1
**Prior handoffs:** none

## Current state

- Epic: `4b7c06d0` — state: backlog (assigned `agent:jit-execution-lead`; it cannot claim while its dependency sinks are open)
- Wave in progress: wave 2 of 7 (wave 1 complete, nothing dispatched for wave 2 yet)
- Children summary: 3 done of 15 impl issues, 0 in_progress, 12 ready/backlog, 0 rejected. Bracket nodes `eebb3fee` (planning) and `3306433d` (breakdown) were already done before this session.
- Plus one issue created during execution: `b1f2f001` (bug) — done.
- Active claims: none. All wave-1 worktrees removed after merge.
- Open escalations: none.
- Progress file: `progress.json` in this directory. It carries the 7-wave plan, per-issue worker assignment, traps, the gate cost model, and `rework_counts`.
- `main` is green at the end of this session: `cargo-ci` all seven steps ✓, `test: 4556 passed, 0 failed, 10 ignored`. `jit validate` exits 0.

## What just happened

- Read the reviewed plan (`plan.md`) and `breakdown.json`; the plan-before-fan-out bracket was already closed, so planning and breakdown were not re-done. Computed 7 topological waves over the 15-issue impl interior and recorded them.
- **Found `main` already red, unrelated to this epic.** `test_shipped_package_decodes_without_edits_and_preserves_identity_hash` pinned the *live* dogfood profile package's identity hash to a literal; that profile packages this repository's own config (including `contrib/gates/*-prompt.md`) as assets, so the earlier prompt edit in `cec90b1c` moved the hash. It would have failed the `cargo-ci` gate on all 15 issues in every wave. The same assertion had already broken once before (gate run `4f8d5399`) and been re-pinned rather than removed. Filed `b1f2f001`, fixed on `d2c8693b` by dropping the literal and keeping the assertion that independently recomputes the released v1 identity algorithm from the package bytes; the synthetic-fixture literals at `package.rs:1462`/`:1467` were left alone because that fixture's bytes are authored in-test. Gates passed, issue done.
- `3db5b63d` (pin/configure nextest foundation) — implemented by codex luna. Verified `.config/nextest.toml` against REQ-01 term by term, confirmed `leak-timeout = "200ms"` really is nextest 0.9.133's default by grepping the installed binary's embedded default config, and confirmed the install and `show-config test-groups` steps precede the job's test step. `cargo nextest show-config test-groups` exits 0 against the committed policy. Both gates passed; done.
- `5b60d40e` (injectable suite duration checker) — implemented by codex luna. I corrected two things before commit: documented *why* the digit-count comparison exists (bash `(( ))` is 64-bit and wraps, so `(( 9223372036854775808 >= 30000 ))` is false and an unguarded compare would let a long value pass the budget — verified empirically), and changed the success-line field from `passed (29999 ms)` to `29999ms` so the line stays key=value parseable, since `summarize_pass` folds it into the persisted gate summary (`scripts/cargo-ci.sh:185`). Boundary verified in isolation: 29999 passes, 30000 fails, leading zeros normalise, 64-bit-wrapping values fail. Both gates passed; done.
- `f5da1112` (single-barrier journal runtime cutover) — implemented by an Opus subagent. Its own worktree gate was green apart from the pre-existing failure above. **code-review FAILED** with one high finding: the journal still declared version 2 while serde silently discarded the removed progress field, i.e. unversioned compatibility (`@/inv/canonical-cutover`). Rework by codex luna direct: bumped `REPOSITORY_JOURNAL_VERSION` to 3 with a comment stating what the boundary means, added `#[serde(deny_unknown_fields)]` to both durable journal structs, swept the version literals, and added a parse-refusal test. I added the missing second test myself — a journal recovery would otherwise complete, with only its version moved back — because the delivered test was refused at parse time and so did not exercise the version bump at all. Proved it guards: with the constant reverted to 2 the test fails. Re-review passed with zero findings; done.
- Merged all three wave-1 branches into `main` with `--no-ff`, ran the leak check (clean), and validated the merged tree — the check no per-issue gate makes, since each worker's gate judged a tree predating the combination.
- Recorded gates as: merge whole wave → `jit gate evaluate <id> cargo-ci` per issue (one execution, three reuses over identical content) → `code-review` per issue. Reclaimed the three merged worktrees.
- Wired `b1f2f001` into the DAG as a dependency of the epic, because an isolated issue makes whole-repo `jit validate` exit 4 and that would fail the epic's own `repo-validate` gate.

## What to do next

- [ ] Dispatch wave 2, all five now `ready`: `47836e2e`, `340be714`, `0369e563` (hard → Opus subagents), `ee809651`, `007f598b` (docs → `codex exec` direct). Specs for the two docs issues are already written in this session's scratchpad; re-derive them if the scratchpad is gone.
- [ ] Conflict note for wave 2: `340be714` adds new crash cases to `crates/jit/src/storage/repository_state_store_tests.rs`, the same file `f5da1112` just edited mechanically and where I added two journal-refusal tests at `:1051` and `:1098`. That is the declared ordered overlap; the new cases go beside the existing ones. No other wave-2 pair shares a file.
- [ ] `007f598b` must remove the last live reference to the removed per-action progress state: `dev/architecture/repository-state-materialization.md:319`. It is the only non-archive file still describing it — verified this session. Its REQ-03 also demands the storage-format, profiles, and guarantees references stay byte-unchanged, so check `git status` shows only that one file.
- [ ] Apply the wave-2 verification policy in `progress.json` (`gate_cost_model`): workers run focused tests plus fmt and clippy, NOT `./scripts/cargo-ci.sh`. The lead's post-merge gate on warm `main` is the authoritative run. This removes ~10 minutes of cold serialized build per issue.
- [ ] After the wave-2 merge batch, reinstall with `./scripts/install-jit.sh` before evaluating any gate, and clear `target/debug/incremental` first.

## Traps — do not repeat these

All eight traps are recorded in full, with file:line evidence and remedies, in the `traps` array of `progress.json` in this directory. Read that array before dispatching. In brief, and in force:

- **Do NOT assume a gate failure is the worker's.** `main` can be red for reasons predating the wave; establish a green baseline on `main` first, then every later failure is attributable. Skipping this cost two full gate runs and nearly sent a worker chasing another issue's defect.
- **Do NOT run an ad-hoc cargo command and then a gate.** `incremental-preflight` (`scripts/cargo-ci.sh:305`) rejects leftover `target/debug/incremental` before compiling anything, and the gate then fails with nothing wrong in the change. `cargo nextest` triggers this too.
- **Do NOT launch concurrent gate runs expecting parallelism.** `cargo-ci` holds a machine-global flock for its whole run (`scripts/cargo-ci.sh:64-68`); they serialize, and queued runs fail on a 1800s lock timeout rather than on their content.
- **Do NOT evaluate a gate after committing to `crates/jit/**` without reinstalling.** The stale-binary guard returns exit 10 and no verdict. Reinstall after the batch's last commit, not before.
- **Do NOT let two gate runs share a log path**, and **do NOT assume stopping an agent stops the gate run it backgrounded** — an orphan keeps the global lock and keeps writing.
- **Do NOT create an issue mid-epic without a dependency edge**; whole-repo `jit validate` exits 4 on an isolated issue and the epic's `repo-validate` gate fails.
- **Do NOT accept a raw-JSON fixture test that asserts only `.is_err()`.** A placeholder `layout_digest` is refused by the layout check before the boundary under test is reached, so the test proves nothing. Build the fixture from a real interrupted transaction, mutate only the field under test, and prove the test fails when that boundary is reverted.
- **Do NOT re-pin a derived hash to a literal.** If a test pins a value derived from this repository's own live configuration, the literal is the defect, not the config edit that moved it. `@/inv/single-source-prose`: a hand-maintained copy of a derived value is a staleness defect. Keep literals to synthetic in-test fixtures.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show 4b7c06d0`
- Reviewed plan: `dev/active/4b7c06d0-test-suite-performance/plan.md` (shared contracts `suite-clock`, `single-barrier-journal`, and the three implementation-produced evidence contracts are binding on later waves)
- Investigation: `dev/active/4b7c06d0-test-suite-performance/investigation.md` (consumer inventories A-D, the `FailurePoint` vocabulary)
- Authoritative graph: `dev/active/4b7c06d0-test-suite-performance/breakdown.json`
- Execution state: `dev/active/4b7c06d0-test-suite-performance/progress.json`
- Committed nextest policy: `.config/nextest.toml`; budget checker and its threshold: `scripts/rust-build-budget.sh`
- Discovered blocker: `jit issue show b1f2f001`
