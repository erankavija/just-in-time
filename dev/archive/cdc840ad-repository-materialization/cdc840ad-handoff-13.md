# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 13

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-22T17:16:03+03:00
**Session number:** 13
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` through `dev/active/cdc840ad-handoff-12.md`

## Current state

- Epic: `cdc840ad` — state: backlog, assigned to `agent:jit-execution-lead`
- Wave in progress: wave 5 of 9
- Children summary: 4 done, 1 in_progress (`49adf23b`), 6 backlog, 0 rejected
- Active claims: `49adf23b` remains assigned to `agent:worker`
- Open escalations: one `rework-exceeded` escalation for the `IssueStore::init` fixture-migration package
- Progress file: `dev/active/cdc840ad-progress.json` (rework count `49adf23b = 2`, open escalation recorded)
- Repository: `main` at `8e5abf1b`; the worktree is intentionally dirty with the uncommitted package and this handoff. Do not discard or reset it.

## What just happened

- Deleted `IssueStore::init`, its JSON partial publisher, its memory no-op, forwarding doubles, predecessor-only tests, and every Rust caller; `ClaimCoordinator::init` remains unchanged.
- Migrated normal fixtures toward canonical `initialize_fresh_repository` plus post-publication layout rediscovery; preserved explicit malformed/partial preimages.
- Updated three live development docs that taught `storage.init()`; historical cdc plan/research/handoff inventories remain intentionally descriptive.
- Package is net-negative (approximately 130 lines deleted) and adds no production compatibility helper or fallback.
- Initial focused checks passed, but full `fast_rules` exposed 16 fixture failures. Rework closed them; `fast_rules` then passed 167/167.
- First independent reviews failed on noncanonical healthy fixtures and stale docs. Rework canonicalized the cited item, scope, document, validation, schema, and label-membership fixtures; reverted an over-broad attempted conversion of the scope suite from memory to JSON.
- Formatting and strict workspace Clippy passed. The library suite passed 2,058 tests before the established unrelated raw-ID dependency failure; with that test skipped, the remaining library suite passed.
- Cumulative `fast_docs_templates` still failed on five package-attributable fixtures: three template-atomicity tests at `template_apply_atomicity_tests.rs:367` and two template-binding tests missing captured `config.toml`. Its sixth policy-text failure is pre-existing at `HEAD` (`HEAD:AGENTS.md` lacks the phrase the untouched test requires).
- Final architectural re-review also found healthy partial-bootstrap assumptions in `default_rules_registry_derivation_tests`, `cli_warnings_integration_tests`, `config_loading_tests`, and multiple server route fixtures. Rework attempt two is exhausted, so correction stopped and escalation was recorded.

## What to do next

- [ ] Resolve the open `rework-exceeded` escalation before editing source.
- [ ] If the invoker authorizes the recommended correction, mark the escalation resolved and reset `rework_counts.49adf23b` before dispatch.
- [ ] Canonically initialize the remaining healthy fast-rules baselines; for the intentional absent-rules case, initialize first, rediscover layout, then delete `rules.toml`.
- [ ] Canonically initialize the healthy JSON server route fixtures and rediscover layout; retain direct bytes only for each route's actual malformed document/symlink target. Do not add a generic legacy-init helper.
- [ ] Repair the three template-atomicity and two template-binding fixtures using canonical setup or the exact required preimage; do not restore the removed forwarding `init` method.
- [ ] Run full `fast_rules`, `fast_docs_templates`, `cli_item_validate`, `cli_query_graph`, and `jit-server`; run formatting and strict workspace Clippy.
- [ ] Re-run independent semantic and architectural review. Acceptance requires PASS with raw bootstrap confined to explicit malformed/partial-state tests.
- [ ] On PASS, commit implementation/docs with `jit:49adf23b`, then commit progress/JIT state separately. Continue deleting the remaining repository-publishing `IssueStore` methods; do not start wave 6 until `49adf23b` is done.

## Traps — do not repeat these

- **Do not treat compile-only coverage as fixture evidence.** The package compiled and Clippy passed while `fast_rules` still had 16 runtime failures and `fast_docs_templates` had five package-attributable failures.
- **Do not replace the memory no-op with an empty physical directory for a healthy JSON fixture.** That preserves partial-bootstrap assumptions and can retain absent-root identity. Use canonical initialization, then rediscover layout.
- **Do not hand-copy `issues/index.json/gates.toml/events.jsonl` for normal setup.** Literal schema-v2 inventories recreate the deleted abstraction in tests. Raw bytes are allowed only when malformed, missing, stale, or recovery state is the subject.
- **Do not convert a broad in-memory suite to JSON to fix one file-backed fixture.** One rework briefly converted all scope-validation tests; it was reverted. Change only the file-backed setup.
- **Do not restore forwarding `init` methods on test doubles.** Give atomicity/binding fixtures their exact canonical precondition instead.
- **Do not attribute the fast-docs policy-text failure to this package.** Neither `AGENTS.md` nor `code_review_policy_test.rs` changed; the required phrase is absent in `git show HEAD:AGENTS.md`.
- **Do not discard the dirty source diff.** It contains the reviewed core deletion, fixture migrations, docs correction, and focused fixes. Inspect with `git diff HEAD` before editing.
- All prior unresolved traps remain in force; especially re-read handoffs 10–12 before dispatching.

## Open questions needing invoker input

- Question: Authorize one narrowly guided correction after the two-attempt rework limit?
  - Context: Core deletion is sound and net-negative, but remaining healthy fixtures still rely on partial bootstrap and five cumulative template tests fail.
  - Options: authorize a counter reset and one fixture-only correction; take over the correction manually; reject `49adf23b` and block the remaining epic chain.
  - Recommendation: authorize the fixture-only correction. The defects are concrete and bounded; rejection would strand an otherwise sound deletion package and block waves 6–9.

## Reference artefacts

- Epic: `jit issue show cdc840ad`
- Active issue: `jit issue show 49adf23b`
- Plan: `dev/archive/cdc840ad-plan.md`
- Progress: `dev/active/cdc840ad-progress.json`
- Prior handoff: `dev/active/cdc840ad-handoff-12.md`
- Current package: `git diff HEAD`
- Failing target: `cargo test -p jit --test fast_docs_templates`
- Passing target after rework: `cargo test -p jit --test fast_rules` (167/167)
- External references: None.
