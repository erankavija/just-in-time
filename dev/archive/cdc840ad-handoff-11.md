# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 11

Status: archived after epic completion on 2026-07-23.

**Date:** 2026-07-20T22:31:45+03:00  
**Session number:** 11  
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` … `dev/active/cdc840ad-handoff-10.md`

## Current state

- Epic `cdc840ad`: wave 5 of 9 in progress; waves 1–4 are done.
- Active issue `49adf23b` (“Migrate all consumers and delete predecessors”): increments 1–7 done; increment 8 remains in progress.
- Repository HEAD: `c9943367` on `main`.
- The worktree is intentionally DIRTY with two uncommitted, disjoint worker slices. Do not discard, reset, rebase, or format across their files before reading this handoff.
- No sub-agent remains active. The gate-registry worker was interrupted on the invoker’s handoff request, then produced the checkpoint below without further edits.
- Formal issue gates have not run. Installed jit must be advanced from `.agents/worktrees/lead-install-clean` to exact clean HEAD before gates.
- Progress ledger: `dev/active/cdc840ad-progress.json`.

## Accepted work since handoff 10

Commits, all for `49adf23b`:

- `98981c4b` — isolated user-global config persistence and deleted the generic repository config-store writer.
- `936a4c27` — migration foundation: mandatory layouts, finalizer-owned typed issue/event publication, atomic batch creation and graph-bypass events, captured document rescan, lifecycle-history repair, shared repository-index codec/validation, retry-stable identity/time, layout-aware fixture cleanup, and source-precedence regressions.
- `58e50eed` — captured issue-local semantic edits for assign/unassign and gate add/remove/manual evidence; retries rederive from fresh captured records while retaining one operation context.
- `c9943367` — real conflict/re-capture evidence for repeated manual gate attestations/failures and corrected canonical CLI prose.

Independent reviews passed for the final lifecycle/index correction, source-priority behavior, captured local issue edits, and the manual-evidence follow-up. Known sandbox-only failures remain claim tests that write `.git/jit/locks/claims.lock` and serve tests that bind loopback sockets.

## Dirty slice A — document add/remove cutover (functionally complete, not independently reviewed)

Owned files:

- `crates/jit/src/commands/document.rs`
- `crates/jit/tests/fast_docs_templates/document_event_log_tests.rs`

Implemented:

- `add_document_reference`/`remove_document_reference` no longer call `publish_ambient_issue_mutation`.
- Add/upsert uses one operation context, fresh sessions on retry, bounded captured asset-closure expansion, final captured-document reparsing, path-based reference lookup, atomic issue+event publication, and true unchanged no-op.
- Remove derives from the captured issue by path and preserves unrelated concurrent fields/references.
- Shared image-backed scanning is reused by the already-captured rescan path.

Worker evidence:

- 4 focused library regressions passed.
- 7 document event-log tests passed.
- 9 CLI document-history tests passed.
- Workspace all-target clippy and diff check passed while the gate slice compiled.
- Delta reported by worker: production `+236`, tests `+189`, total `+425`. This is a maintainability warning: review should demand that the closure/retry code is irreducible and should delete duplication where possible before accepting.

Next action for this slice: independent adversarial review for mixed snapshots, closure expansion, no-op/audit behavior, retry identity, and code size. Commit separately by staging only the two owned files after PASS.

## Dirty slice B — gate definition registry cutover (implemented, NOT green)

Owned files:

- `crates/jit/src/commands/gate.rs`
- `crates/jit/src/commands/gate_cli_tests.rs`
- `crates/jit/src/commands/mod.rs`
- `crates/jit/src/repository_state/mod.rs`
- `crates/jit/src/repository_state/mutation.rs`
- `crates/jit/tests/common/harness.rs`
- `crates/jit/tests/fast_docs_templates/planning_preset_tests.rs`
- `crates/jit/tests/fast_docs_templates/template_apply_tests.rs`
- `crates/jit/tests/fast_docs_templates/template_apply_atomicity_tests.rs`

Implemented:

- `add_gate_definition`, `define_gate`, `update_gate`, and `remove_gate_definition` now edit a registry derived from captured `gates.toml` on every retry.
- One retained `MutationContext`; typed `EditGateRegistry`; registry serialization in declarations; one plan combines registry bytes, finalized definition event, and the complete configured producer/materialization set.
- Four migrated methods no longer call `save_gate_registry` or `append_event`.
- Duplicate/missing errors, checker merge semantics, projection coupling, recovery atomicity, retry identity/time, and doctest behavior were covered.

Passing checkpoint evidence:

- Gate-related library tests: 62 passed.
- CLI gate-update tests: 16 passed.
- CLI gate-modification tests: 12 passed.
- Updated `update_gate` doctest passed.
- Focused projection/recovery/retry tests passed; compile succeeded; the slice’s `large_enum_variant` warning was fixed.

Known blocking fixture defect:

- File-backed fast-docs fixtures seed config using `storage.write_repo_file(".jit/config.toml", "")`. When `JsonFileStorage::root()` is already the data root, that writes relative to the repository parent rather than `<data-root>/config.toml`. Result: `captured image has no .jit/config.toml`; full `fast_docs_templates` had 46 genuine fixture failures plus 2 known claim-lock sandbox failures.

Safest resume:

1. In concrete JSON fixtures write `std::fs::write(storage.root().join("config.toml"), "")`.
2. In generic atomicity fixtures, use the physical root write only for `is_file_backed()`; retain aggregate-image seeding for memory.
3. Rerun focused gate tests, affected fast-docs tests, full `fast_docs_templates`, fmt, diff check, and workspace clippy.
4. Independently review capture completeness, proposed-declaration consistency, complete producer coupling, retry/no-op behavior, and whether `EditGateRegistry` stays purpose-specific rather than becoming a generic authored-bytes seam.
5. Commit separately by staging only this slice’s owned files after PASS.

## Hard burn-down and remaining scope

After accepting the document slice, `publish_ambient_issue_mutation` should have 13 production callers:

- dependency graph operations: 4
- issue update/claim/release/gate-blocking: 4
- bulk update: 1
- label add: 1
- shared state transition: 1
- validation fixes: 2

Two additional ambient full-record updates remain: delete cascade and automated gate-result publication. Captured document rescan and the new document add/remove paths are intentionally direct `UpdateIssue` finalizer users because they derive from their owned session image.

Remaining major work, recommended order:

1. Finish/review/commit the two dirty slices above.
2. Graph slice: dependency add/remove/batch/reduction plus transitive-reduction repair, derived from one complete captured issue graph.
3. Transition slice, sequential because of overlap: replace `apply_state_transition`; migrate issue update/claim/release/gate-blocking and bulk update without ambient config/rules/plan reads.
4. Local validation residue: label add and type fix.
5. Bind automated gate checker evidence to captured issue/definition/input identities; reject stale evidence.
6. Capture config/rules/gates for single and batch creation.
7. Whole-operation cutovers: preset application/custom preset creation, template application, archive artifacts+reference changes+retirement. Delete compensation/rollback machinery in the same slices.
8. Init/startup recovery, repository-contained export/snapshot, rules/default-rule writers, `.jit/` prefix adapters, and validation fallback removal.
9. Delete `IssueStore` mutation methods/implementations/forwarding doubles and legacy transaction/recovery APIs; run live-tree absence scans.
10. Only then install exact HEAD and run `cargo-ci`, `code-review`, `mcp-ci`, `docs-mechanical` sequentially.

## Maintainability guardrails

- Do not grow `CapturedIssueMutation` beyond simple issue-local assign/gate operations. Graph, document, transition, external evidence, template, and archive operations need purpose-specific pure derivation.
- Do not retain `publish_ambient_issue_mutation` as a compatibility path; caller count must monotonically reach zero.
- Do not add generic byte/action/delta callbacks. Gate/config edits may serialize structured declarations only inside purpose-specific finalizers invoking the complete producer set.
- One `MutationContext` is created before each retry loop and reused across fresh recovered sessions/images.
- Mutation-time validation reads config/rules/gates/plan documents from the same captured image; executor `OnceLock` caches are read-only conveniences.
- `apply_state_transition` is a bad abstraction: extract pure captured transition derivation and delete its persistence flag/callback behavior.
- Template/archive compensation code is deletion debt, not an architecture to preserve.
- Pair every additive migration with immediate caller/API deletion. Production growth without predecessor deletion is a failed slice.
- Keep `IssueStore` on course to read/query-only; `RepositoryStateStore` is the sole repository publication capability.

## Binding rulings and resolved escalations

All rulings 1–16 in handoff 10 remain binding. Additional resolved decisions/evidence this session:

- Historical lifecycle repair changes only `first_ready_at`, `claimed_at`, and `done_at`; it preserves captured `updated_at` and every non-lifecycle field.
- One pure `RepositoryIndex` codec/validator is shared by ordinary reads and mutation membership finalization. Aggregation uses per-id priority local → HEAD → main for both active and tombstone states; existing malformed/I/O sources fail instead of masquerading as absence.
- Manual gate pass/fail is explicit evidence, not an idempotent field edit: repeated valid operations record a fresh event/timestamp; retries within one operation retain identity/time.
- Gate addition has one canonical declared+Pending behavior; the undeclared/no-status test convenience was deleted.

There are no open invoker questions. The rework ledger is at 2 for `49adf23b`; do not silently exceed it—use the execution-lead escalation protocol if another reviewed package exhausts its bounded retries.

## Reference artefacts

- Plan: `dev/archive/cdc840ad-plan.md`
- Progress: `dev/active/cdc840ad-progress.json`
- Prior detailed mechanism/ruling ledger: `dev/active/cdc840ad-handoff-10.md`
- Active issue: `jit issue show 49adf23b --json`
- Installer worktree: `.agents/worktrees/lead-install-clean`
