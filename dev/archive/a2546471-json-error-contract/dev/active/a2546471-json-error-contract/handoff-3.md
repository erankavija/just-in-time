# Handoff — Structured failure reporting across the machine-readable CLI surface (a2546471) — session 3

**Date:** 2026-07-28T14:17:52+03:00
**Session number:** 3
**Prior handoffs:** `dev/active/a2546471-json-error-contract/handoff.md`, `dev/active/a2546471-json-error-contract/handoff-2.md`

## Current state

- Epic: `a2546471` — state: backlog; assigned to `agent:jit-execution-lead`
- Wave in progress: Wave 4 of 8; all three implementations are merged, but issue gates are incomplete
- Children summary: 23 done, 3 in_progress, 9 backlog/ready, 0 rejected
- Active claims: `72835d56`, `a193a7cc`, and `dbe2f2a8` are assigned to `agent:worker` since 2026-07-28T10:13Z; no worker process is modifying them
- Open escalations: `dbe2f2a8` exhausted `MAX_REWORK_ATTEMPTS=2` after its third cargo-ci attempt exposed another stale pre-feature assertion; invoker guidance is required
- Progress file: `progress.json` in the epic's artifact directory, `jit doc dir a2546471 dev/active` (reflects the above)

## What just happened

- Completed, gated, reviewed, and closed all six Wave 3 issues; advanced the persisted plan to Wave 4.
- Dispatched the three Wave 4 implementations in isolated worktrees using difficulty-matched models, merged worker commits `aa47bd6e`, `4a0f2a81`, and `7aa1160e`, and verified every integration/follow-up commit with `scripts/verify-commit-builds.sh`.
- Delivered `72835d56` invocation-form parity coverage, `a193a7cc` dual-form exit projection coverage, and `dbe2f2a8`'s typed top-level JSON failure renderer; the integrated `cli_issue` target passed 295 tests before gate rework.
- Reworked `dbe2f2a8` attempt 1 after cargo-ci found four double-document `validate --json` failures. JSON validation reports now terminate after their handler-owned document while preserving the existing stderr diagnostic and exit 4; all 111 `cli_item_validate` tests passed.
- Reworked `dbe2f2a8` attempt 2 after cargo-ci activated two stale query-label assertions. Added public `InvalidLabelPatternError`, registered `INVALID_LABEL_PATTERN` at exit 2, preserved the tailored suggestion, regenerated `docs/reference/error-codes.md`, and passed vocabulary/classifier/query-label coverage.
- The third cargo-ci attempt passed fmt, zero-warning clippy, provenance, build budget, and incremental-state, but failed one `cli_repo_workflow` assertion that explicitly demands empty stdout for corrupt `config get --json`; runtime correctly emitted the new required `PARSE_ERROR` envelope.
- Persisted all three failed cargo-ci runs (`20f8606a`, `02f8ef59`, `daac8001`) and recorded the open exhausted-rework escalation at main commit `b9315715`.
- Coordinated with Claude planning lead through `/tmp/jit-forum`. A synthesizer briefly raced the hold and wrote two untracked e204e63d planning files; Claude removed them without staging, confirmed ownership, and now keeps all output outside the checkout. Claude declined a later commit-only window because adversarial review is still changing those files; main remains free of Claude changes.

## What to do next

- [ ] Read the invoker response to the `dbe2f2a8` exhausted-rework escalation.
- [ ] If the invoker provides guidance/resets the counter, update `config_get_tests::test_get_corrupt_config_toml_json_is_not_invalid_argument` to parse exactly one stdout envelope, resolve its code as registered `ErrorCode::ParseError`, assert status parity and retained stderr, then run the focused `cli_repo_workflow` target.
- [ ] Record the escalation resolution and reset `rework_counts.dbe2f2a8` before committing the test correction with `jit:dbe2f2a8`; verify the commit in isolation.
- [ ] Ensure `target/debug/incremental` is empty, install with `./scripts/install-jit.sh` from a clean provenance tip, then evaluate both configured `dbe2f2a8` gates and perform the six-tier lead review.
- [ ] If `dbe2f2a8` passes, transition it Done, commit its gate/state evidence, and gate/review/close `72835d56` and `a193a7cc` sequentially before advancing Wave 4.
- [ ] Keep Claude outside the checkout until a clean block is available. Its preferred later sequence is final plan+manifest commit, plan-review on `7eecace8`, then batch-create; each mutation window must end clean before epic gate work resumes.
- [ ] After Wave 4 closes, validate the graph, reclaim only merged Wave 4 worktrees, advance `progress.json` to Wave 5, and continue the existing eight-wave plan.

## Traps — do not repeat these

- **Do not treat handler-owned validation JSON as an unrendered propagated failure.** `validate --json` already prints its full report before the integrity error is classified; allowing it to reach `emit_top_level_json_error` appended a second document and broke four tests. Exit from the handler after printing while retaining the stderr diagnostic (`crates/jit/src/main.rs`, commit `2b955ad9`).
- **Do not collapse malformed query labels into generic `INVALID_ARGUMENT` or restore an unregistered literal.** Existing consumers require `INVALID_LABEL_PATTERN` and its suggestion; the correct path is the dedicated typed error plus registered enum member (`717429f4`).
- **Do not preserve stale tests that explicitly require an empty payload under `--json`.** The remaining config-get failure is the opposite of `dbe2f2a8` REQ-01: the new structural top-level contract requires one registered envelope. Update the semantic assertion; do not suppress the runtime envelope.
- **Do not continue rework without invoker guidance.** `dbe2f2a8` has used both permitted retries, and the third evaluation failed. `references/escalation-policy.md` requires an invoker decision even though the next correction is narrow.
- **Do not let Claude's planning process and epic gate mutations overlap.** The first hold arrived after its synthesizer had already written `dev/active/e204e63d-derived-package-sources/{plan.md,breakdown.json}`. Claude removed them and now holds outside the checkout; grant explicit bounded windows and verify `git status` before provenance installs.
- **Do not assume subagent worktree writes stay isolated in this environment.** One Wave 4 worker patch reached main despite a worktree prompt. The lead captured the exact diff, applied it to the intended worktree, restored main, and ran the leak checker. Re-run the leak check before reclaiming Wave 4 worktrees.
- **Do not leave regenerable incremental caches under `target`.** Manual builds previously left a non-empty 384 MB `target/debug/incremental`, which failed the gate's hygiene check. Use `CARGO_INCREMENTAL=0` and inspect/remove only that exact cache before gate evaluation.
- Re-read both prior handoffs' trap sections; their provenance-install, external-review authorization, exact survey, `serve`-row, and cumulative-finding warnings remain active.

## Open questions needing invoker input

- Question: Should `dbe2f2a8` receive a reset rework counter and one further narrowly guided correction?
  - Context: Three cargo-ci attempts found three different stale boundary assertions; the current sole failure explicitly expects no JSON where REQ-01 requires a registered `PARSE_ERROR` envelope.
  - Options: authorize one additional rework cycle with the test correction above; take over the issue manually; or reject `dbe2f2a8` (which prevents completion of epic `a2546471`).
  - Recommendation: authorize one additional narrow rework cycle because production behavior now matches the reviewed criterion and only the obsolete assertion conflicts.

## Reference artefacts

- Epic: `jit issue show a2546471 --json`
- Issue: `jit issue show dbe2f2a8 --json`
- Design docs: `dev/active/a2546471-json-error-contract/plan.md`
- Planning docs: `dev/active/a2546471-json-error-contract/breakdown.json`
- Progress: `dev/active/a2546471-json-error-contract/progress.json`
- Prior handoffs: `dev/active/a2546471-json-error-contract/handoff.md`, `dev/active/a2546471-json-error-contract/handoff-2.md`
- Failed gate evidence: `.jit/gate-runs/20f8606a-2dbc-4224-8482-256aae70c72f/result.json`, `.jit/gate-runs/02f8ef59-94e1-4e13-af46-2897068b910b/result.json`, `.jit/gate-runs/daac8001-78e4-4107-86c9-7b2991da793a/result.json`
- Wave 4 integrated commits: `aa47bd6e`, `4a0f2a81`, `7aa1160e`; rework commits: `2b955ad9`, `717429f4`
- Claude coordination: `FORUM_DIR=/tmp/jit-forum`, identities `agent:execution-lead` and `agent:planning-lead`
