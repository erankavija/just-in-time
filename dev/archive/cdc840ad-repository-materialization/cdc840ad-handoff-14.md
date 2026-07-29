# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 14

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-22T18:38:53+03:00
**Session number:** 14
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` through `dev/active/cdc840ad-handoff-13.md`

## Current state

- Epic: `cdc840ad` — wave 5 of 9 remains active.
- Active child: `49adf23b` — final publisher-deletion package is uncommitted and blocked after rework attempt two.
- Repository: `main` at `377dc5e7`; the worktree intentionally contains 63 modified files with a net-negative package (1,506 additions, 2,239 deletions). Do not reset or discard it.
- Open escalation: one `rework-exceeded` entry in `dev/active/cdc840ad-progress.json`.
- Rework count: `49adf23b = 2`.

## Completed in this session

- Committed the broad `IssueStore::init` deletion and canonical fixture migration as `e670fe53`.
- Committed the prior fixture-correction escalation resolution as `618fb331`.
- Deleted `restore_issue_verbatim` in reviewed commit `377dc5e7`.
- In the remaining dirty package, deleted the final repository-publishing `IssueStore` methods and backends, migrated fixtures to typed mutations or explicit aggregate seeders, and obtained a clean structural absence scan for the seven predecessor writer names.
- Corrected three cumulative bugs exposed by the cutover: non-Git root authority, claim/coordinator lock ordering, and standalone profile derivation/no-op behavior.
- Formatting and strict workspace Clippy passed. Escalated MCP tests passed (54 unit, 9 integration). The full Rust workspace passed every target reached until the pre-existing `fast_docs_templates` exact-policy-text assertion.

## Final independent review result

Both required reviews failed, exhausting the two-attempt limit:

1. `JIT_DATA_DIR=../.jit` is compared to the discovered root without lexical normalization. From a nested directory it selects the correct data root but the wrong worktree root, so profile assets publish below the child. Relative and absolute spellings of the same path behave differently.
2. `Commands::coordinates_claims_first` groups nested command enums instead of exhaustively matching every leaf. Its test samples only part of the CLI inventory, and `dep rm` is incorrectly claim-first even though `RemoveBatch` explicitly disables lease enforcement.
3. Public `RepositoryIndex` exists only to support integration fixtures. Deserialization can create duplicate or overlapping membership, while public serialization claims to emit a canonical index without validating those invariants. This is a test-convenience leak and an incomplete public codec.
4. Two profile producer/finalizer comments still assign audit bytes or omit derived rules/schema closure, contradicting the implementation.

Verified sound: `IssueStore` is read/query-only; listed legacy writers are absent; remaining references are historical or command-level; fixture index seeding no longer duplicates raw JSON field inventory; the package remains net-negative; `git diff --check` passes.

## Required decision

The execution-lead policy requires invoker direction before a third correction. Recommended option: authorize one final tightly bounded correction and reset the counter. Scope it only to normalized root equivalence plus regression, exhaustive exact claim-lock classification (excluding `dep rm`) plus complete inventory coverage, a legitimate internal fixture boundary or invariant-validating complete index codec, and the two stale comments. Then rerun focused/full validation and one independent final review.

Alternatives are to take over the dirty correction manually or reject `49adf23b`, which blocks waves 6–9 and the epic.

## Traps

- Do not restore any deleted publisher, `IssueStore::init`, or compatibility helper.
- Do not broaden root discovery semantics beyond equivalent-path normalization.
- Do not patch only the sampled claim commands; make the classification structurally exhaustive at leaf level.
- Do not retain a public production codec solely for test convenience unless its public material contract is complete and serialization enforces invariants.
- Do not apply the one-line `AGENTS.md` policy-text clarification until source rework is authorized; it was intentionally held outside the reviewed dirty package.
- Preserve all current dirty changes.

## References

- Progress: `dev/active/cdc840ad-progress.json`
- Package: `git diff HEAD`
- Current head: `377dc5e7`
- Semantic reproduction: nested non-Git repo with `JIT_DATA_DIR=../.jit`
- Architecture review focus: `main.rs`, `cli.rs`, `repository_state/index.rs`, `repository_state/initialize.rs`
