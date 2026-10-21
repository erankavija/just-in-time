## Epic Complete: Fix correctness defects found by the v1.0 code audit (8376299c)

**Started:** 2026-10-20
**Completed:** 2026-10-21
**Assignee:** agent:jit-execution-lead

### Summary

All 15 audit-filed correctness defects and one defect found during release verification are fixed on main. Each fix has a regression test that failed on the pre-fix code. Every documented reproduction shows the corrected behaviour on a release build.

### Metrics

| Metric | Value |
|---|---|
| Children completed | 16 / 16 |
| Waves executed | 4 (wave 4 holds the bug found during verification) |
| Rework cycles | 6: review-driven for 7d1bfe89, bedf0962, 05d1b34e, c0d0c8bf, 16d63038; a scope redo for d92f54aa |
| Escalations | 6, each resolved by the user (through the steward) or by the steward |
| Sub-agent dispatches | 21, including reworks and one verification agent |
| Issues created during execution | 1 (b5faef84) |

### Success Criteria

- [x] REQ-01: Every defect filed under this epic is fixed, and each fix is covered by a regression test that fails on the pre-fix code. Delivered by all 16 children. Each worker recorded its red run. Two partial cases:
  - For d92f54aa, the red evidence is the pre-fix binary accepting the removed keys.
  - For 7d1bfe89, the lexer tests ran red; the CLI exit test did not run red.
- [x] REQ-02: Every documented reproduction in the child issues produces the corrected behaviour on the release build. See `repro-verification.md`: 15 corrected and 1 N/A. The N/A is 6ff14f21, a race with no deterministic reproduction; its policy test is cited instead.
- [x] REQ-03: No fix introduces a parallel variant of an existing mechanism. Duplicated logic converged on one source in each of these fixes:
  - 16d63038: `IssueIdPrefix` in the domain layer; every resolver takes it.
  - bedf0962: one enforcement-mode resolver.
  - c0d0c8bf: one label-pattern validator, delegating to the canonical label validator.
  - 05d1b34e: the bulk path reuses `check_auto_transitions` and a shared terminal-transition flag.
  - aa423d02: reuses `QueryFilter`.
  - 6ff14f21: one `EnvGuard` and an in-process test-mode flag.
  - d92f54aa: one built-in stale-threshold constant.

### Wave Execution Log

**Wave 1:** 10 issues. 2d10f1dd, d7d0d0db, d92f54aa, 0fa1e09e, cd2ed136, 7d1bfe89, 34999aad, 514df7d3, 6ff14f21 and d23c58c0. Covered mutation atomicity, lease eviction, the coordination timing keys, the server data directory, graph export, lexer UTF-8, ripgrep argv, hook paths, the env race and orphan validation.
**Wave 2:** 4 issues. bedf0962, aa423d02, c0d0c8bf and 05d1b34e: lease-mode resolution, the claim-next filter, search label wildcards and the bulk terminal cascade.
**Wave 3:** 1 issue. 16d63038: one issue-id resolver for every entry point.
**Wave 4:** 1 issue. b5faef84: server search over the default nested `.jit`.

### Key Decisions

- Waves were sequenced by expected file overlap; the children have no sibling dependencies. d7d0d0db and d92f54aa shared one worker because both edit `claim_coordinator.rs`.
- Wave 3 was dispatched while c0d0c8bf still waited on a load-only cargo-ci rerun. Its code and reviews were final, and every file overlap that motivated the wave split had already merged.
- A worker's first attempt that failed review was escalated one model tier where possible: sonnet to opus for 7d1bfe89, 05d1b34e and c0d0c8bf.
- Workers run focused checks only. The lead runs the full gate on main under the host build turnstile, because of disk and load limits.
- Load-only cargo-ci failures were rerun with `--force` under the exclusive lock. No quiet-host budget failure occurred.

### Escalations

- **d92f54aa scope:** the user (through the steward) chose to delete the three inert coordination timing keys rather than wire them up. The issue was retitled and its criteria replaced, and the worker was redirected.
- **2d10f1dd REQ-02:** the steward ruled that the `transition_blocked` refusal event is kept, and REQ-02 was amended to match.
- **bedf0962:** `config show` also misreports the claim-coordination limits. The steward routed this to epic 842b0ac6 as its REQ-26.
- **Lease-eviction defect class:** `renew_lease` and the acquire paths drop evictions. The steward routed this to epic f8a6b298's REQ-06.
- **b5faef84 scope:** the steward accepted the bug under this epic, on condition that the fix stays minimal, because epic 0f8efc37 replaces server issue search.
- **cargo-ci budget:** the suite clock rose from about 22 s to 29 s on a quiet host. The steward holds this for the user and made a binding ruling for the interim.

### Issues Discovered During Execution

- b5faef84: "Search endpoint returns no issue matches for the default nested data directory". Created after wave 3; the REQ-02 release reproduction run exposed it.

### Holistic Quality Notes

- Three reviews failed on the same pattern: a fix that satisfied the named case while a sibling path kept the old behaviour.
  - 05d1b34e triggered the cascade on the request instead of the transition.
  - c0d0c8bf's validator was laxer than the canonical one.
  - 16d63038 left some entry points reading before validating.
  
  Prompts that name the whole entry-point family, and resolvers that are type-driven (`IssueIdPrefix`), closed these.
- Several fixes removed code: `scripts/test-hooks.sh`, `add_label`, duplicate resolvers, the three coordination keys and `JIT_ENFORCE_LEASES`.
