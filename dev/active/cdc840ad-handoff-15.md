# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 15

**Date:** 2026-07-22T19:07:59+03:00
**Session number:** 15
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` through `dev/active/cdc840ad-handoff-14.md`

## Current state

- Epic `cdc840ad`: wave 5 of 9 remains active.
- Child `49adf23b`: implementation is committed at `1a661786`; issue remains `in_progress`.
- Worktree is clean.
- One open shared-infrastructure escalation blocks the final workspace test and issue gates.
- Rework count: `49adf23b = 1` after the invoker-authorized reset.

## Completed

- Deleted the final repository-publishing `IssueStore` APIs/backends and migrated consumers/fixtures.
- `IssueStore` is now read/query/session-control only.
- Exact scans are empty for `save_issue`, `restore_issue_verbatim`, `save_gate_registry`, `append_event`, `write_repo_file`, `save_gate_preset`, and `save_gate_run_result`.
- Corrected equivalent relative/absolute `JIT_DATA_DIR` root authority with a nested profile regression.
- Made claim-first lock ordering leaf-exhaustive and exact; both CLI dependency batch verbs remain repository-first.
- Kept `RepositoryIndex` crate-private, validated serialization invariants, and routed integration fixtures through one explicit in-memory semantic seeder.
- Corrected standalone profile derived-repair/no-op behavior and its public audit/authority contract.
- Independent final semantic and architecture reviews pass.
- Commit `1a661786` is net-negative: 1,877 additions, 2,258 deletions.

## Validation

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- MCP: 54 unit and 9 integration tests pass.
- Full Rust workspace: all targets reached pass until `fast_docs_templates`; that target has 197 passed, 6 ignored, and one pre-existing policy-text failure.
- Failing assertion: `test_code_review_prompt_rejects_blanket_public_api_examples`.
- Missing exact AGENTS phrases: `Do not require an example for every public API` and `tautological examples`.

## Required decision

Changing `AGENTS.md` is shared repository agent policy, so the execution-lead escalation policy requires invoker authorization. Recommended: add both missing phrases to the existing Public API documentation bullet as a one-line semantic clarification. The existing bullet already establishes the same non-obvious-only policy and already contains the third required phrase, `one type- or module-level walkthrough`.

If authorized, apply only that line, rerun `fast_docs_templates` and the full workspace suite, commit separately, then evaluate all four gates on `49adf23b` and continue waves 6–9.

## Traps

- Do not weaken, skip, or delete the policy assertion.
- Do not treat the exact-phrase failure as package-attributable; it exists at the parent commit.
- Do not modify any source in the accepted publisher package unless a gate reports a new concrete defect.
- Keep the AGENTS clarification in a separate shared-policy commit.
- Preserve prior handoff traps, especially the prohibition on restoring publishers or compatibility helpers.
