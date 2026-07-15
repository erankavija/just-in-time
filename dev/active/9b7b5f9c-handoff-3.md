# Handoff — Versioned repository profiles and portable JIT dogfood setup (9b7b5f9c) — session 4

**Date:** 2026-07-15T21:05:00+03:00
**Session number:** 4
**Prior handoffs:** `dev/active/9b7b5f9c-handoff.md`, `dev/active/9b7b5f9c-handoff-2.md`

## Current state

- Epic: `9b7b5f9c` — state: backlog
- Wave in progress: wave 1 of 9
- Children summary: 0 done, 3 in_progress, 9 backlog, 1 completed breakdown node outside the implementation waves
- Active claims: `92039d9c`, `866a5bbd`, and `be542b98` by `agent:worker`; prerequisite `d3709cc6` by `agent:worker`
- Open escalations: none awaiting input; the invoker explicitly authorized continued overlay rework and brought `d3709cc6` into scope
- Progress file: `dev/active/9b7b5f9c-progress.json` (reflects the above)
- Main: clean at `3e02db29`; post-dispatch leak check passed
- Preserved branches:
  - `worktree-agent-be542b98-r5` at `48526373891aebb4a6df31db49c1f1f749cfd538`, clean and fully worker-verified
  - `worktree-agent-d3709cc6` at `854dff4504ba0fa0a272042132e43365a5b0c2eb`, clean WIP checkpoint with remaining full verification

## What just happened

- Recorded the invoker's renewed blanket authorization and completed overlay rework attempts 4 and 5.
- Attempt 4 commit `4595d534`, merged as `3e4f9c5c`, introduced `RepositoryValidationFailure`, retained simultaneous structural and rule diagnostics, and passed its exact-merge isolated build.
- Attempt 4 gates passed `cargo-ci` and `repo-validate`; code-review run `be860123-be85-4169-83c2-e173e642faff` found one high blocker: the built-in `repository_validation` gate still used legacy storage-backed validation.
- Attempt 5 commit `48526373` routes file-backed built-in repository validation through the exact `RepositoryView` pipeline, preserves custom JIT roots and partial failures, adds an explicit storage-backend capability, and covers injected overlay disagreement, stale legacy caches, missing index, structural+rule findings, placeholder warnings, and exit 4.
- Attempt 5 worker verification passed 1,748 library tests (4 ignored), 104 `cli_item_validate`, 245 `cli_issue`, 163 `fast_rules`, 127 `cli_gate`, workspace all-target clippy, formatting, and diff checks. It is not merged and no gates have run for it.
- Advanced the formerly external dependency chain: `73482aa1` is Done with all gates passed; the invoker explicitly directed completion of ready blocker `d3709cc6`.
- Claimed `d3709cc6` in commit `3e02db29` and preserved its remediation at WIP commit `854dff45`.
- `d3709cc6` baseline audit found 10 vulnerabilities and 7 denied warnings. The checkpoint upgrades direct `git2` 0.18→0.21, `quick-xml` 0.40→0.41, and `notify` 7→8, plus precise patched lock versions for `bytes`, `rustls-webpki`, `tar`, `time`, `anyhow`, and both `rand` lines. No ignore or allowlist was added.
- `d3709cc6` final audit currently passes with zero findings over 376 dependencies; all-feature clippy and a targeted real-repository git metadata regression pass. The git2 0.21 fallible text accessors retain prior best-effort omission semantics.
- Stopped at the invoker's handoff request before the prerequisite's full all-feature build/test matrix.

## What to do next

- [ ] Re-read all prior handoff trap sections, then verify main is clean and both preserved worktrees match the commits above.
- [ ] Merge `worktree-agent-be542b98-r5` into current main with `--no-ff`; run the skill leak check first and `scripts/verify-commit-builds.sh` immediately after the merge.
- [ ] Reinstall JIT from the resulting clean HEAD, clear generated target state, then evaluate `be542b98` gates in order: `cargo-ci`, `repo-validate`, `code-review`. Build the cumulative closure table across runs `27811bd8`, `a24ea0c4`, and `be860123` before accepting review.
- [ ] Resume `worktree-agent-d3709cc6`; run final `cargo fmt --all -- --check`, `cargo build --workspace --all-targets --all-features`, `cargo test --workspace --all-features`, and `cargo audit -D warnings`; clean its local target.
- [ ] After the prerequisite matrix passes, amend the isolated WIP commit into a final `jit:d3709cc6` commit (or otherwise produce a clean final branch without merging a WIP-labelled commit), then leak-check, merge, and verify the exact merge commit.
- [ ] Evaluate `d3709cc6` gates `cargo-ci`, `jit-validate`, and `code-review`; perform success-criteria and holistic review, mark it Done, and commit JIT state.
- [ ] Rerun `866a5bbd dependency-audit`; when all six gates pass, retain its prior cumulative review closure.
- [ ] Only when `92039d9c`, `866a5bbd`, and `be542b98` all pass every gate, mark the three wave-1 children Done together, validate the repository, advance progress to wave 2, and dispatch `eceffc17`.
- [ ] Once `d3709cc6` is Done, update `0aec3b1e` from `blocked_external_dependency` to pending/ready according to live JIT state; wave discipline still prevents starting it before waves 1 and 2 finish.

## Traps — do not repeat these

- Prior handoff traps remain in force; especially preserve cumulative review findings, deterministic-gates-before-review ordering, clean target state for `cargo-ci`, and exact-HEAD JIT reinstall discipline.
- **Do not stop after fixing the ordinary CLI validation caller.** Review run `be860123-be85-4169-83c2-e173e642faff` proved the built-in `repository_validation` gate was a second supported caller. Attempt 5 routes it through the same view pipeline.
- **Do not infer a file-backed store from `index.json` existence.** A partial repository may legitimately be missing the index and must fail through the view. Attempt 5 adds `IssueStore::is_file_backed`; preserve its missing-index regression.
- **Do not merge overlay attempt 5 without rerunning all three gates at the new HEAD.** Its worker matrix is green, but its code has never been evaluated by issue gates.
- **Do not merge `854dff45` as-is.** It is a handoff WIP checkpoint and REQ-02 still lacks the explicit all-target/all-feature build and full workspace all-feature test evidence.
- **Do not treat `cargo audit -D warnings` as vulnerability-only.** The baseline also denied seven warnings, including unmaintained/unsound crates; the remediation intentionally removes every warning without an ignore list.
- **Do not assume git2 0.21 text accessors still return `Option`.** `Remote::url` and `Reference::shorthand` are fallible; the checkpoint helpers deliberately convert errors to omission to preserve prior snapshot behavior.
- **Do not install JIT while main has concurrent uncommitted state.** One install embedded `dirty=true` and was unusable as provenance evidence. Wait for a clean state commit, reinstall, and confirm `jit --version` matches HEAD.
- **Do not reuse a gate result after main advances.** The stale-binary guard correctly rejected this during concurrent work. Commit the current gate evidence, reinstall at the new HEAD, and rerun the remaining gate.
- **Run cache-writing project scripts with the required escalation.** `cargo-ci` and exact-commit verification initially failed only because their user-cache temp roots were read-only in the sandbox.

## Open questions needing invoker input

None.

## Reference artefacts

- Epic: `jit issue show 9b7b5f9c`
- Plan: `dev/active/9b7b5f9c-plan.md`
- Progress: `dev/active/9b7b5f9c-progress.json`
- Prior handoffs: `dev/active/9b7b5f9c-handoff.md`, `dev/active/9b7b5f9c-handoff-2.md`
- Overlay attempt 5: branch `worktree-agent-be542b98-r5`, commit `48526373891aebb4a6df31db49c1f1f749cfd538`
- Overlay failed reviews: `.jit/gate-runs/27811bd8-1fe5-40e0-9877-7ff21a86c745/result.json`, `.jit/gate-runs/a24ea0c4-91d6-484e-adb1-a6566a98dd87/result.json`, `.jit/gate-runs/be860123-be85-4169-83c2-e173e642faff/result.json`
- Advisory prerequisite: `jit issue show d3709cc6`; branch `worktree-agent-d3709cc6`, WIP commit `854dff4504ba0fa0a272042132e43365a5b0c2eb`
- Attempt-4 exact merge: `3e4f9c5c`
