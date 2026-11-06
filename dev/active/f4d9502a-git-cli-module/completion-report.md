## Epic Complete: Reach git through one storage-owned CLI module instead of libgit2 (f4d9502a)

**Started:** 2026-10-24
**Completed:** 2026-11-06
**Assignee:** agent:f4d9502a-lead

### Summary

Every git access in the workspace now goes through one storage-owned module that runs the git CLI against the invocation's selected checkout (@/charter/D-21). libgit2 and its OpenSSL linkage are gone from the dependency graph, the build-policy check rejects their return, and adopter and contributor documents describe the git executable requirement.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 58 / 58 (all `epic:git-cli-module` issues done) |
| Waves executed | 10 |
| Rework cycles | 26, across 21 issues (from `progress.json` `rework_counts`) |
| Escalations | 5 recorded escalation threads, plus steward rulings listed below |
| Sub-agent dispatches | 48 recorded in `progress.json`, including reworks; earlier sessions may be undercounted |
| Issues created during execution | 2 (783b33fa, 053b109e) |

### Success Criteria

Mapping is by `satisfies:f4d9502a/REQ-NN` labels on done children; the epic `holistic-review` gate verified each criterion against source, tests and CI evidence.

- [x] REQ-01 — no git2 dependency, no git2/libgit2-sys in the lock, no libgit2/OpenSSL linkage — 6fdb2d53, 78cb05ed (plus 053b109e advisory remediation that unblocked 6fdb2d53's audit gate)
- [x] REQ-02 — exactly one storage-owned module starts git — 81664a6a, 34851181
- [x] REQ-03 — every git call uses the configured executable against the selected worktree — 81664a6a, 3f8da8c4, 24d3b460
- [x] REQ-04 — layout facts computed at most once per distinct checkout context — d68150e2, 07663984, 5046408a
- [x] REQ-05 — exactly one branch-name lookup — 4ba18af4, c140921c
- [x] REQ-06 — read commands read no git objects in a primary checkout; at most one process in a linked checkout — 7297f922, 1acfa592, d7cce445
- [x] REQ-07 — counting stand-in proves the process budget for `query available` and `issue show` — 7297f922, 7aafec83
- [x] REQ-08 — lease validation lists worktrees at most once per run — 437907da, f649c144
- [x] REQ-09 — gate execution gets commit and branch evidence through the module — 3f8da8c4
- [x] REQ-10 — document show, history, diff, asset checks, snapshot export and pinned reads go through the module; commands hold no git code — a273667c, 1acfa592, 7cd27db8, 933ff852, d9ba0029
- [x] REQ-11 — the module exposes only production-called operations — 34851181, e6ec58f3
- [x] REQ-12 — core commands work outside git and without an executable; claims fail typed — 23b48cf0, f649c144
- [x] REQ-13 — manifest comments, rustdoc and documentation about libgit2 corrected — 6fdb2d53, caceed7d, 23b48cf0

Checkpoints 34bb375a (libgit2 removal) and d28888f0 (branch, worktree and base facts) each passed `cargo-ci` and `holistic-review` with zero findings.

### Wave Execution Log

**Wave 1:** 4 issues — shared git test fixtures, Dockerfile and command groundwork.
**Wave 2:** 4 issues — the git module contract and its first primitives.
**Wave 3:** 9 issues — head lookup, layout probing, worktree listing and early consumer migrations.
**Wave 4:** 9 issues — process-budget tests and further consumer migrations.
**Wave 5:** 4 issues — layout-context bounds and remaining storage reads.
**Wave 6:** 10 issues — split-root code/data authority (5046408a), commit path reads with the Git >= 2.42 floor (a273667c), the shared original-expression-then-peel resolver (c140921c), archive pinned evidence (d9ba0029).
**Wave 7:** 4 issues — document show, snapshot export, resolver-bridge removal.
**Wave 8:** 2 issues — document diff and the command-reference statements.
**Wave 9:** 1 issue — libgit2 removal (6fdb2d53), preceded by advisory remediation 053b109e.
**Wave 10:** 4 issues — git-executable documentation, contributor documents, module surface trim, native-linkage build check.

### Key Decisions

- Landed every change directly on `main` through `land.sh` (or an equivalent locked no-ff merge when a foreign untracked file blocked the script), each followed by a forced `cargo-ci` on the merged result as merge evidence.
- Ran a non-fail-fast workspace suite outside the sandbox before every forced full gate after 5046408a's first full-gate failure exposed a hidden merge interaction; the steward's lesson (run main-changed storage tests before forcing an integrated gate) was adopted.
- Resolved the `Cargo.lock` for 053b109e in the lead session because Codex sandboxes have no registry network; the steward accepted this.
- Corrected two stale issue citations in briefs rather than widening scope: 78cb05ed's fixture tests went into the existing `build_profile_policy_tests.rs` owner; 34851181 migrated seven test-module callers that its REQ-01 explicitly covers.
- Made `CommitResolution::into_version` crate-visible so 933ff852 reused the shared adapter instead of copying it; the steward later confirmed convergence visibility raises are the lead's call.
- Dispatched cd603c8e and 933ff852 out of wave order under the steward's standing disjoint-footprint rule, recorded in `progress.json`.

### Escalations

- **Resume after park** — the user lifted the wind-down park and approved the Git >= 2.42 floor and Debian Trixie runtime; a273667c and c140921c proceeded.
- **5046408a selected-A boundary** — design R1 FAIL closed by an amended retained-root design and independent R2 PASS; a full-gate failure from a semantic merge with 4334df62 led to steward option A (one bounded rework) and option X (Io displays its source message once at top level; once-only chain rendering withdrawn after the renderer audit). Landed after four rework cycles.
- **c140921c footprint** — steward authorized a fifth file (`repository_state_store_tests.rs`) and a sixth (`check_links_tests.rs`) for argv-shape assertions only, with counts unchanged.
- **cd603c8e wave order** — steward approved out-of-order dispatch and granted the standing rule.
- **6fdb2d53 dependency audit** — pre-existing RustSec advisories on main failed the blocking audit; steward chose remediation 053b109e (semver-compatible lockfile updates only, no ignores) and declined narrowing the gate. The `syn 3` transitive duplicate was accepted conditional on unchanged build-footprint budgets, which passed.

### Issues Discovered During Execution

- 783b33fa — Allow an explicit acting identity for issue release (before wave 7; release-actor failures in contract tests without ambient identity).
- 053b109e — Clear the Rust dependency advisories reported against the current lockfile (wave 9; advisories published after d3709cc6 blocked the audit gate on every branch).
- f532daf2 — Lock guards verify the identity of the lock file they hold (filed on v1.1 for the steward; outside this epic, from the 5046408a design advisory).

### Holistic Quality Notes

- Merge interactions, not individual changes, caused most late failures: 5046408a × 4334df62 (symlink capture), 5046408a × 4d834432 (redo apply), c140921c × check-links (argv-shape assertions). Running the full workspace suite on the integrated branch before forcing the configured gate caught each one early afterwards.
- Behaviour change flagged to the user via the steward: a git-mode `jit snapshot export` now fails with exit 3 when a referenced document or asset is missing at the export commit (7cd27db8, per its REQ-02); the CLI reference states it.
- The epic holistic review raised one low advisory: `handoff-5.md` states disk and inode figures without recorded command output. It is historical narrative and affects no criterion.
- Follow-up candidate, not filed: `doc check-links` could reuse one cat-file Types session for commit resolution and HEAD asset presence, saving a process (recorded in `progress.json`; steward asked to file only on a D-19/D-20 reason).
