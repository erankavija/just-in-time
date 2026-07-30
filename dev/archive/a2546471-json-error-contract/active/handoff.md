# Handoff — Structured failure reporting across the machine-readable CLI surface (a2546471) — session 1

**Date:** 2026-07-28T03:39:38+03:00
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `a2546471` — state: backlog; assigned to `agent:jit-execution-lead`
- Wave in progress: Wave 1 of 8 is fully landed; resume at Wave 2
- Children summary: 15 done, 0 in_progress, 2 ready, 18 backlog, 0 rejected
- Active claims: No active child leases. Wave 2 issues `6ebd3d03` and `8cfe8699` are Ready and unclaimed; the epic assignment remains with `agent:jit-execution-lead`.
- Open escalations: None.
- Progress file: `progress.json` in `dev/active/a2546471-json-error-contract` reflects the above.

## What just happened

- Loaded the reviewed plan, breakdown, investigations, project policy, and the full issue payloads; reconciled the 35-node implementation graph into eight dependency waves.
- Claimed and dispatched all 15 Wave 1 issues in isolated worktrees, selecting frontier/high reasoning for the error-code API and complex namespace surveys and balanced models for smaller surveys.
- Delivered `b6917d8f`: the error-code vocabulary is a 20-member enumerable enum with derive-bound completeness, exhaustive status and description maps, typed JSON construction, explicit legacy statuses, and fallible text resolution.
- Closed two cumulative enum review findings at the maximum two rework attempts. Final `cargo-ci` passed 3,995 tests, formatting, zero-warning clippy, provenance, build budgets, and incremental-state hygiene; final code review passed with zero findings.
- Produced and linked 14 TOML failure-lever fragments covering 99/99 reflected `--json` command paths. `serve` is intentionally represented by three rows for start/stop/status; config has two source-only exemptions and standalone has the version exemption.
- Ran an independent cross-fragment audit: exact reflected-path coverage, schema fields, full argv vectors, setup-token consistency, namespace containment, and document links all passed.
- Reworked survey records where review found reproducibility defects: issue primary-worktree setup, doc/config/profile canonical fields/vectors, wrapper invariant recipe, standalone hooks failure, and gate preset-apply failure text.
- Merged all 15 worker branches sequentially into main. Because the repository lacks `scripts/verify-commit-builds.sh`, verified every merge commit with an equivalent temporary helper that archived the named commit into an isolated tree and ran offline `cargo check --workspace --all-targets --locked`; all 15 passed.
- Evaluated every required Wave 1 gate on provenance-matched code. All 14 survey `code-review` gates and both enum gates are passed; all 15 Wave 1 issues are Done.
- Confirmed `6ebd3d03` and `8cfe8699` are Ready with no unmet dependencies.
- Removed only the 15 merged Wave 1 worktrees after proving every branch tip is an ancestor of main, then pruned worktree metadata. The branches and commits remain recoverable.

## What to do next

- [ ] Confirm main is clean and load this handoff plus `progress.json`; do not redispatch Wave 1.
- [ ] Resume Wave 2 by claiming `6ebd3d03` and `8cfe8699` for `agent:worker`, committing the claim/progress mutation, and dispatching isolated worktrees from the same clean main tip.
- [ ] Use `gpt-5.6-sol` with high reasoning for both Wave 2 issues: emitted-code registration changes the central vocabulary/status contract; registry assembly must merge 99 measured rows unchanged and build typed parser-reflection equality checks.
- [ ] Treat the Wave 2 pair as parallel-safe after preflight: `6ebd3d03` owns the vocabulary and current emission classifications; `8cfe8699` owns the committed test registry, typed deserialization, and reflection reconciliation. Recheck their actual touched paths before dispatch.
- [ ] For `8cfe8699`, merge the 14 survey fragments without reinterpretation. Preserve all recorded content, the three `serve` branch rows, and the three source-only exemptions; assert the unique reflected path set is exactly 99.
- [ ] After Wave 2 workers commit, run the leak check, integrate sequentially, and verify each exact merge commit before further commits. Recreate the temporary isolated verifier if `/tmp/jit-verify-commit-builds.sh` no longer exists.
- [ ] Install the dogfood binary with `./scripts/install-jit.sh` from a clean, committed provenance tip before evaluating gates. Commit gate evidence before reinstalling if evidence dirties the checkout.
- [ ] Update `progress.json` after Wave 2, then continue dependency waves 3–8; do not advance a blocked issue early.

## Traps

- **Do not pass `--full` to `jit issue show`.** The installed CLI treats `show` as the full single-issue payload and rejects that flag; use `jit issue show <id> --json`. Use `--full` on supported bulk query/search commands, matching the invoker's request for full issue data.
- **Do not run manual Rust validation with incremental compilation enabled in an issue worktree.** Manual `cargo test` created `target/debug/incremental` and the stale-child fixture's incremental directory, causing otherwise-green `cargo-ci` runs to fail only their hygiene check. Use `CARGO_INCREMENTAL=0` for every manual cargo compile/test/clippy invocation before the authoritative gate.
- **Do not install a provenance binary from a worktree with uncommitted gate evidence.** `install-jit.sh` embeds `dirty=true`, and stale-binary checks will refuse it. Commit the lead-owned gate audit trail first, then install from the clean tip.
- **Do not trust a `git:absent` hooks probe under `/tmp`.** This environment exposes `/tmp/.git`, so `find_git_dir` reaches that sentinel and reports or attempts hooks-directory creation instead of the true no-repository error. A source-based review caught the contamination; an unsandboxed disposable `/var/tmp` repo reproduced `HOOKS_INSTALL_ERROR: Not in a git repository (no .git directory found)`. The two temporary hooks accidentally installed under `/tmp/.git/hooks` and all repro repositories were removed.
- **Do not rely on a repository merge-verifier script being present.** `scripts/verify-commit-builds.sh` is absent even though the workflow protocol names it. Session 1 used `/tmp/jit-verify-commit-builds.sh`, which archives the named commit, extracts it into a disposable tree, and runs offline workspace/all-target `cargo check` with a shared `/tmp` target. Recreate that equivalent if the temporary helper has expired; never skip per-merge verification.
- **Do not assume a semantically plausible expected failure is exact.** Gate review found two records whose prose differed from actual JSON: preset apply uses `errors[0].error = "Issue not found: …"`, and true git-absent hooks reports `Not in a git repository`. Re-run the exact recorded vector under the exact setup and compare both payload and exit status.
- **Do not collapse the three `serve` rows into one during registry assembly.** Reflection has one `serve` path, but the reviewed survey explicitly measures its start, stop, and status branches as three rows sharing that path. Set equality uses unique paths; the probe registry retains all branch invocations.
- **Do not lose prior findings when reviewing rework.** Read all gate runs and recheck the cumulative finding list. The enum required two successive F1 closures before the final zero-finding pass; issue, wrapper, standalone, and gate surveys also have prior failed review evidence that must remain in history.

## Open questions

None.

## Reference artefacts

- Epic: `jit issue show a2546471 --json`
- Design: `dev/active/a2546471-json-error-contract/plan.md`
- Authoritative breakdown: `dev/active/a2546471-json-error-contract/breakdown.json`
- Progress: `dev/active/a2546471-json-error-contract/progress.json`
- CLI investigation: `dev/active/a2546471-json-error-contract/cli-investigation.md`
- Failure-lever measurements: `dev/active/a2546471-json-error-contract/levers/*.toml`
- Wave 2 issues: `jit issue show 6ebd3d03 --json`; `jit issue show 8cfe8699 --json`
- Gate history: `jit gate status <issue-id> --all --json`; current findings: `jit gate status <issue-id> code-review --findings --json`
