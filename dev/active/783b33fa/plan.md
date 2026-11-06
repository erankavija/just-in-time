# Explicit acting identity for issue release

**Issue:** 783b33fa  
**Status:** Steward-approved bounded correction; four-file implementation, Sol/high, isolated worktree. This task belongs to the existing Git CLI epic outside its original manifest, with no epic requirement coverage credit.

## Contract

The authoritative criteria are @/issue/783b33fa/requirement/REQ-01 through @/issue/783b33fa/requirement/REQ-04. Add a release facade accepting the existing Assignee type. Keep the invocation-resolving facade and one captured lifecycle release producer. A supplied actor differs from the previous assignee; audit attribution cannot default to that assignee or an anonymous actor.

Preserve convenience ordering: resolve IssueIdPrefix through storage, then resolve invocation identity using the existing worktree authority, then publish the captured Release request. Do not evaluate an actor-resolution argument before prefix lookup. The explicit facade resolves the prefix and supplies its actor directly. State validation, no-op behavior, lease/policy/session checks, derived state and event publication keep their existing owner and order.

## Evidence and existing inputs

On unchanged main f64105f313d55b83b8832b1d67f2cd30d31cb9fd, each existing contracts test fails alone with `Cannot release: no acting identity for the audit trail`: `fast_issue::mutation_result_tests::test_release_issue_returns_updated_issue` and `fast_issue::test_no_coordinator::test_issue_lifecycle_works_without_coordinator`. Main stayed clean, with source index fingerprint b667e44cfa3d536b8ae54aace1db62372bf2474a7a463a7b999ffb86dcd154ee unchanged. Matching worker runs fail identically; the seven relevant implementation/fixture files are byte-identical.

The child process removes all JIT inputs, uses an empty owned XDG_CONFIG_HOME, disables global/system Git configuration, and leaves HOME unchanged. The original JSON/log evidence is retained under `/tmp/f4d9502a-lead/5046408a-actor-baseline-*` and `5046408a-actor-source-audit.json`. No ambient identity was supplied to turn these failures green.

The generic identity resolver supports injected inputs, but issue release has no explicit actor facade. The private lease-release callback is a different operation and audit path. Existing in-memory fixtures deliberately supply no Git identity. No new actor provider, fake Git checkout or test-only bypass is needed.

## Exact implementation footprint

| File | Change |
| --- | --- |
| crates/jit/src/commands/issue.rs | Explicit typed release facade; preserve existing lookup-before-identity convenience ordering. |
| crates/jit/src/commands/mod.rs | Existing private release producer accepts selected Assignee; captured lifecycle/event path remains singular. |
| crates/jit/tests/fast_issue/mutation_result_tests.rs | Supply actor in existing release scenario; retain persisted-result assertion; verify distinct audit actor and convenience lookup precedence in this owner. |
| crates/jit/tests/fast_issue/test_no_coordinator.rs | Supply actor explicitly; retain in-memory/no-coordinator and cleared-assignee assertions. |

A fifth owner requires evidence and steward approval before editing. No CLI flag, identity vocabulary, resolution priority, storage format, Git floor, coordinator or actor-setting mechanism changes.

## Validation and delivery

Retain the reproduced RED. Develop the new facade/actor relationship test first; run the affected cases alone and together under the same sanitized inputs, then full contracts without fail-fast, affected existing release/identity owners, formatting and all-target Clippy. Existing lifecycle and resolver mechanism suites retain ownership; test only the new input/wiring properties here.

Lead runs the required configured gates and review, canonical main landing and actual merged checks, then closes the task. The live dependency graph makes this a prerequisite of split-root work. That work remains frozen while this task edits commands/mod.rs; after this task lands and is Done, refresh its source into the frozen split-root worktree before resuming that separate correction. This task neither resets its retry limit nor changes unrelated user holds.
