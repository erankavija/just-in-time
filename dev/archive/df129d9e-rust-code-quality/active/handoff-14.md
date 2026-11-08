# Handoff — Dispatch layering, executor surface, and build hygiene in crates/jit (df129d9e) — session 14

**Date:** 2026-11-05
**Session number:** 14
**Prior handoffs:** `handoff.md` and `handoff-2.md` through `handoff-13.md` in this directory. Their unresolved traps remain binding.

## Current state

- Epic: `df129d9e` — state: backlog.
- Wave in progress: 9 of 13, held at timing enforcement. Authorized independent work in later waves has been completed; consult the refreshed `progress.json` and actual DAG.
- Children summary: five direct dependencies Done; the remaining direct build-hygiene story is Backlog behind the intentionally held timing chain. All independent planned work is complete.
- Active claims: no active worker leases; epic and held timing issue remain assigned to `agent:df129d9e-lead` (claimed 2026-10-20 and 2026-11-04 respectively). Assignment ages at handoff: df129d9e: about 15 days 5 hours; f487aeef: about 0 days 11 hours. Completed assignments are historical bookkeeping, not active execution.
- Open escalations: none awaiting a new answer. The user explicitly keeps dynamic CPU sharing and defers timing enforcement; `f487aeef` and its dependent host-lock trial `fba8d478` remain held. General autonomy approval does not revoke that specific decision.
- Progress file: `progress.json` in this directory. This lead owns only this epic; other leads own Git, frontend and steward work.
- Production ledger: verified attributable core delta **+184**, including the final documentation generator citation's +1 line; server cleanup is −8 separately. The approved core ceiling is +375. Do not claim that the whole epic is net negative.

## What just happened

- Completed split-debug `75b46be9` and suite consolidation `b22aabce`: active executable bytes fell 31.43685% in the first change; the second reduced integration targets from 10 to 3 and executables from 14 to 7, measuring 1,177,002,600 → 673,455,960 bytes. Both added zero core production lines.
- Completed result ownership `fb38bf1f` (+37 core), executor story `f35c86de` (−3), dispatch story `c06daf50` (0), and RequiresGit consolidation `487f45a9` (−1), with final reviews and sequential merged checks. The current contract keeps result types with their defining layer and forbids commands importing output.
- A complete cross-child audit found four defects after narrower reviews passed: overbroad dynamic findings projection, a restored intermediate renderer, stale search status prose, and stale active-plan emitter prose. All are closed.
- Completed error cleanup `0eca332a`: source `88c419187` and octal correction `43d8ad971`; merge `97d00504e`, merged Cargo `5d29627b`, review `766aceb2`, closure `d3fe25f71`. Removed 21 core and 8 server lines. A positive producer-backed code slice serves admission/schema; real subprocess bindings cover reachable statuses and preserve the exact census. Two mapping tests no longer wait one second for a timer.
- Completed renderer repair `ab168296`: merge `67f4514c3`, merged Cargo `244d875e`, review `93f3da67`, closure `85e56e0cd`; zero core delta.
- Completed main.rs guard `f9977efd`: shared counter, ceiling 40 from observed 37, existing shell fixtures, both transitive gate-input roots corrected. Merge `da2245869`, merged Cargo `15d42f63`, review `15c7cc86`, closure `3abf77ce3`; zero core delta.
- Completed Rustdoc repair `755964b2`: 111 compiler-confirmed diagnostics across 52 files, all source edits in Rustdoc comments. Merge `e2925fcd8`, merged Cargo `98eb3137`, review `59b5ca56`, closure `520d97e08`. The actual merged private-items Rustdoc command passed in 9.78 seconds with no link diagnostics. Linked `dev/active/755964b2/verification.md` preserves source, command and output. The epic's Rustdoc pitfall is resolved; zero core delta.
- Completed canonical CLI documentation `0546fc82`: the external installed-dispatcher defect was repaired by `dc048377`, without a waiver. The final full audit also corrected three collection examples, their graph summary, and false universal envelope wording in the canonical generator. Approved D-02 records that small footprint extension. Final source `b4d26070a`, mechanical `be51aa00`, review `f24a5166`, merge `49c2582ea`, closure `b2ebfdb67`. Direct merged Cargo passed 4,974 tests and 25 doctests plus fmt, Clippy, exclusions and budgets. Core delta +1, measured against the merge's first parent.
- Completed error-boundary story `fe3ad1b0`: both stale current narratives fixed in `e08e28738`; complete six-tier audit passed, including all six hard criteria and cumulative findings. Merge `33e7e2d93`, actual merged Cargo `b59bcca8`, review `6c6439db`, closure `09cd937c8`; zero core delta.
- Completed output/schema story `46fe7593`: actual Cargo `8b15acf2` passed 4,974 tests and 25 doctests; configured review `552067d6` and complete six-tier acceptance passed. Its only attributable changes were gate records; merge `3ed24cd50`, closure `efd289afc`, validation passed. No duplicate full Cargo run was claimed or performed for that tracker-only integration; zero core delta.
- Used difficulty-selected agents: Astra high for semantic error/binding repair, Sol high for complete cross-child and final story audits, Luna high for bounded compiler-reported Rustdoc corrections. Configured reviews ran in isolated worktrees concurrently with other work. Full merged checks stayed sequential under the existing shared-host coordination policy.
- Reclaimed completed worker worktrees through the canonical cache-harvesting helper. No host runner or dotfiles were edited. Blocked 061/38 source and policy changes were not included through any branch.

## What to do next

- [ ] Preserve the explicit hold on `f487aeef` and `fba8d478`. Timing should identify tests that wait for timers, not charge host-token queue time or reduce dynamic sharing. Existing diagnosis: cargo-pool-run waits for a jobserver token inside nextest's per-test timer; fixed invocation budgeting was declined. No new implementation has been authorized by elapsed time.
- [ ] If the user changes that decision, resume the held work through its actual requirements and gates, coordinating shared runner changes with the steward. Do not silently remove the host-lock trial's literal prerequisite.
- [ ] After the held chain completes, finish `a08c4b38`, `fb2dc5c3`, then the epic's repo validation and holistic review. Do not call this handoff a completion report or mark the epic Done early.
- [ ] Refresh the DAG, actual gate state and line ledger before resuming. Other epic leads continue changing main; their commits are not attributable to this epic.

## Traps — do not repeat these

- **Do not declare every ErrorCode on a dynamic findings leaf.** `ALL` projected unreachable status 6 and broke the exact subprocess-row census. The positive actual-producer slice and real witnesses close it; no blacklist, fabricated witness or weakened census.
- **Do not wait for timers to test error mapping.** Established lock ownership plus timeout zero proves mapping immediately; FIFO synchronization reaches the nested mutation boundary. The underlying locker tests own elapsed-time behavior.
- **Fixture placement changes error wrapping.** Checker-side chmod reached generic wrapping; pre-invocation registry denial reaches typed PermissionDenied. The classifier was not changed to satisfy the fixture. Permission zero must be written `0o000`; decimal zero failed Clippy after tests passed.
- **Trace transitive checker inputs.** Both build-budget-selftest and cargo-ci-selftest execute production-lines.py. The initial direct-only audit missed the latter; approved root addition `52c2ec0fb` and final gates close it.
- **Do not recreate the removed Rust fixture harness.** The steward reconciled f997 REQ-04 to the existing shell self-test. Its under/exact/over cases are the approved owner.
- **Use the default committed production counter, without flags or checkout.** A worker's optional-mode estimate of −17 was actually −1 in the canonical mode. Measure each attributable merge against its first parent, not all changes from other epics since a shared baseline.
- **Current tracker descriptions are not history.** An earlier RequiresGit sweep omitted three live descriptions. Historical events, reviews, investigations and handoffs remain history; current requirements must agree with the implementation.
- **Narrow reviews do not replace cross-child acceptance.** The complete audits caught projection/census integration, renderer regression, status prose, and later example/generator contradictions. Preserve all prior findings and check them cumulatively.
- **Check complete examples against their contract.** Three JSON arrays had one item but counts of two or three; the graph summary also named a nonexistent ready node. All nine complete count-bearing JSON examples now agree. No new checker was added.
- **Generated prose is repaired at its generator.** The exit reference falsely said every invocation had an envelope. Change schema.rs, use the existing regeneration script, and cite the canonical parser/child contract; do not hand-edit the projection or duplicate its exception policy.
- **Search current test comments and architectural prose.** fe3's first formal review failed on a removed macro reference and a future classifier-move sentence despite passing functional gates. Both exact prose fixes are in `e08e28738`; the failed formal report and later passing evidence are preserved.
- **Doctests do not verify Rustdoc links.** Facade moves changed what `Self` meant without changing the comment's blame. The real private-items build initially reported 111 diagnostics, including public-to-private errors even with private items enabled. Fix compiler-confirmed references without changing visibility, adding wrappers or suppressing lints.
- **A private target does not justify removing every link.** The capture helper is referenced from a private caller, so its qualified canonical link is legal and literally required. Rustdoc review F1 rejected a code span; `c26f2bf2c` restores the link.
- **Put non-gate proof in tracked linked evidence.** Rustdoc review F2 could not see a /tmp build log. The linked verification document now records both branch and main source/command/results. Preserve concrete parser names and CLI placeholders while correcting references.
- **Do not run a dependent gate after a failed merge.** A documentation timestamp conflict was followed by a PARSE_ERROR before any checker. Check exit status, resolve and commit, then evaluate. Merge progress conflicts field by field and preserve unrelated concurrent tracker updates.
- **Use bounded tracked-text searches.** A broad filesystem search scanned generated archived JavaScript and produced a huge truncated result. Recover with a complete tracked Markdown/Rust search before claiming the audit is complete.
- **Installed tooling must support the helper's new flags.** Canonical dispatch briefly refused `--field` before dc048's installation completed. Wait for the owner to install it; do not bypass the dispatcher. Its prior jq dependency was a real M8 defect, not grounds for a waiver.
- **A Git merge of Done dependencies can leave stale Backlog.** For 0546, native Ready then claim repaired the derived workflow state. Do not edit dependency JSON or run a broad validation fix. The approved dependency addition used native `--reduce` for the one redundant edge.
- **Pending gate reports use the findings envelope.** `gate status-all` with one pending gate puts its report in `error.details`; blindly reading top-level `results` raised KeyError. Interpret the documented envelope, not a presumed success shape.
- **Keep reviews and cheap checks outside Cargo locks.** Build commands use existing coordination; `flock -o` prevents daemonized children inheriting the lock. Cancel only a proven own childless queued wrapper. Do not nest JIT mutations inside `jit worktree exec`; lifecycle calls happen outside its lock.
- **A leak checker may list legitimate tracker writes from another lead.** Inspect ownership; never restore or delete another lead's evidence. Pinned configured reviews use their own `.jit` with JIT_DATA_DIR unset; preserve actual failures and superseded passes, never fabricate reuse.
- **Approval rejection is action-specific.** The steward reopened independent work after ancestry/source proof; the rejected 061 merge and 38 policy were not retried indirectly. Any new rejection must be reported and resolved on its own terms.
- Earlier handoff traps remain in force, including no old close-issue.sh, no blanket counting exclusions, no long-lived epic integration branch, and no unreviewed scope or gate changes.

## Open questions needing invoker input

None newly required. The timing and host-lock work remains intentionally deferred by the user's specific decision; general autonomy approval does not reopen it.

## Reference artefacts

- Epic and dependency authority: `jit issue show df129d9e`, `jit graph deps df129d9e --depth 0`, and actual per-issue gate records.
- Plan, manifest and execution snapshot: `plan.md`, `breakdown.json`, `progress.json` in this directory.
- Rustdoc proof: `dev/active/755964b2/verification.md`, linked to its completed issue.
- Session audit/check logs: `/tmp/jit-df129d9e-session14/`, including cumulative story audits, final lead reviews, prior failed reports and source-pinned merged check logs. Durable result IDs and dispositions are also recorded in progress and JIT.
- Steward thread `01a1a886-d199-7093-b096-7acbab6adb48`; Git lead `01a1a88a-3bda-7302-8fdf-0662e2095cc1`; frontend lead `01a1a892-b1aa-7fd1-8454-dfae9bb53d06`.
- Canonical dispatch/land/reclaim helpers: `.agents/skills/jit-execution-lead/scripts/`. Worker temporary root override: `/mnt/nvme/jit/tmp/df129d9e-workers`; keep build caches on NVMe and short owned test TMPDIRs outside the repository.
