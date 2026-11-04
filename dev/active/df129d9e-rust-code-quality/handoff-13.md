# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 13

**Date:** 2026-11-04
**Session number:** 13
**Prior handoffs:** [handoff.md](handoff.md), handoff-2.md through [handoff-12.md](handoff-12.md)

## Current state

- Epic remains backlog, assigned to `agent:df129d9e-lead`; its children carry execution.
- Session wave 1 is complete. Wave 2, `75b46be9`, awaits a criterion decision after implementation and full branch CI. Consult the live graph for child counts; the progress file reconciles the older wave entries with tracker state.
- `75b46be9` is in_progress, assigned to the lead, with no advisory lease. Its clean worktree is `.agents/worktrees/agent-75b46be9`, branch `worktree-agent-75b46be9`, commit `1daddafd1`. It has not landed on main. No worker or build remains running.
- Open approvals: the executable-reduction floor and the D-59 ownership wording reconciliation described below.
- [progress.json](progress.json) records models, reviews, measurements, and the timing decision. Session-specific scheduling is under `session_13`; older wave status fields were reconciled, but their original sequencing remains historical.

## What just happened

- User resumed this epic, requesting model choice by difficulty, simplification, and preferably net-negative production lines.
- `f855d631`: Luna implemented test-profile incremental=false, policy coverage, and documentation. A clean workspace build plus an actual targeted test with CARGO_INCREMENTAL unset left zero incremental file bytes. Branch and main cargo-ci passed (5,037 tests, 25 doctests); independent review passed without findings. Closed; worktree reclaimed.
- `5f12265a`: Sol implemented incremental exclusion and coherent newest-generation pruning, including existing pool artifacts and the cp fallback. Review R1 found omitted WebAssembly suffixes. Rework generalized suffix handling; 128 shell assertions passed. Full branch/main CI passed, and review R2 passed without findings. Closed; worktree reclaimed through the new mechanism.
- Rust production deltas for both completed issues are zero. The crate remains 73,631 production lines; the inherited epic-attributed total remains +171 against the recorded +375 ceiling.
- `75b46be9`: Sol measured the untouched starting commit `efc34b1c5` before enabling unpacked debug information. Executable bytes fell from 1,716,674,768 to 1,177,006,296: 31.436850%, below the hard 40% criterion. The existing expected-panic test retains workspace source-line backtraces. A 1,234 MiB budget provides 9.935078% headroom. Full branch CI passes (5,038 tests, 25 doctests, fmt, clippy, budgets). Measurements and reproduction are in the branch's build-budget README. No gate or criterion has been bypassed or amended.
- Astra audited remaining simplification opportunities: move existing result types inward for `fb38bf1f`, retain output renderers, and avoid parallel DTOs/conversion layers. Its unmeasured estimate was -20 to +40 production lines. Dead-code retirement remains a useful reduction opportunity.
- Astra reproduced the host runner defect with a private pipe: `cargo-pool-run` waits for a token before executing the test, inside nextest's timeout. The user declined fixed invocation budgeting and explicitly chose dynamic sharing with timing enforcement deferred. No dotfiles were changed. The user clarified that limits should expose slow test execution, including timer waits, rather than cap bandwidth or count queue wait.

## What to do next

- [ ] Resolve the pending `75b46be9` decision. Proposed amendment: REQ-02 changes from at least 40% to at least 30%, accepting the measured 31.44% from the specified setting. Proposed description is in `/tmp/jit-df129d9e-session13/75b46be9-proposed-description.md`; regenerate from the live issue if that temporary file is gone. Do not apply without approval.
- [ ] If approved, record the decision, update the branch README's unmet-criterion statement, merge current main into the branch, and finish the normal landing/gates/review lifecycle. The branch has full CI evidence, but its required tracker gates remain pending. If 40% is retained, investigate further without silently adding compiler/linker policy or weakening backtraces.
- [ ] Then execute `b22aabce` (suite consolidation), measured against the tree it starts from. Do not count its reduction toward the split-debug task's separate measurement.
- [ ] Continue the remaining epic tasks per the graph and handoff-12. `fb38bf1f` and `487f45a9` may overlap output/error surfaces; serialize or merge carefully. Keep the line ledger honest and prefer deletion/unification.
- [ ] Keep `f487aeef` and dependent work held under the user's explicit timing decision. A clean solution preserving dynamic sharing needs nextest-side acquisition before its attempt clock; the pinned runner API has no ready handshake. Do not substitute fixed reservations or a duplicate shell timeout-policy interpreter.
- [ ] Resolve the D-59 criterion/plan inconsistency before the epic's final review. Do not claim completion against conflicting criteria.

## Traps — do not repeat these

- **Do not assume a filename suffix list covers Cargo artifacts.** Review R1 of `5f12265a` found `.wasm` absent. The final mechanism groups arbitrary suffixes and resolves dep-info companions through their artifacts.
- **Do not lose empty directories when replacing cp with a filtered file traversal.** Preliminary review caught that regression; the final cp fallback preserves directory metadata through a directory-only tar stream, covered by the fixture suite.
- **Do not use an unqualified Rust test name with `--exact`.** The first incremental measurement selected zero tests. The corrected substring invocation ran one policy test and still measured zero bytes; only that run is evidence.
- **Do not infer host process absence from sandboxed ps output.** The sandbox sees its own namespace. Escalated inspection found a long-running sccache daemon even when sandboxed ps found none. This session used the supported no-sccache opt-out; it did not start or stop the daemon.
- **Do not assume unpacked achieves the planned percentage.** It left substantial line/range tables linked on Rust 1.97.0. Measurements, not the issue's estimate, establish the delivered reduction.
- **Do not treat the old timing handoff as an unanswered user repair request.** The defect is confirmed, and the user has now explicitly chosen to preserve dynamic sharing and defer enforcement. No runner fix was installed.
- Prior handoff traps remain in force unless superseded. Use host locks with `flock -o`, external worker TMPDIRs, main-only lifecycle writes, and lead-owned git mutations.

## Open questions needing invoker input

- Approve changing `75b46be9` REQ-02 to a 30% floor? Recommendation: accept the measured 31.44% and tighter 1,234 MiB budget without adding compiler/linker mechanisms just to reach the prior estimate. The question was asked; no answer was received before this checkpoint.
- Approve reconciling epic REQ-01 and the plan's `handler-response-ownership` rule with D-59? Proposed REQ-01: “No success response body is constructed in crates/jit/src/main.rs; handlers return typed command or domain results, and rendering and envelope construction belong to the output layer. Commands do not depend on the output layer.” Recommendation: remove the conflicting output-owned-type requirement, preserving dependency direction. The same obsolete wording appears in story `c06daf50`; include it in the final consistency review. No criterion was amended this session.

## Reference artefacts

- Epic: `jit issue show df129d9e`; [plan.md](plan.md), especially D-54 and D-59.
- [progress.json](progress.json); [handoff-12.md](handoff-12.md).
- Split-debug branch: `1daddafd1`; `dev/benchmarks/rust-build-budgets/README.md` in that branch contains the durable measurements and backtrace reproduction.
- Completed gate evidence: tracker runs for `f855d631` and `5f12265a`. Cache review F1 is closed in `fbc670e2`.
- Temporary command logs: `/tmp/jit-df129d9e-session13/`. They are supplemental; durable measurements and tracker gates are authoritative.
