# Handoff — Transactional repository materialization and derived-state coherence (cdc840ad) — session 17

**Date:** 2026-07-23T05:42:41+03:00
**Session number:** 17
**Prior handoffs:** `dev/active/cdc840ad-handoff.md` through `dev/active/cdc840ad-handoff-16.md`

## Current state

- Epic `cdc840ad`: waves 1–6 of 9 are complete; wave 7 is next and was not started.
- `661d6be2` is Done. All 11 required gates pass; external code-review run `aee5b723-8284-4ecd-b093-56ce631240b3` returned zero findings.
- Wave-7 children `a3a788f3`, `b1621508`, and `ed3e773c` are Ready and unclaimed.
- Frozen reviewed candidate: `f8d75181`. The final completion/evidence commit is the handoff commit created after this file.

## What just happened

- Consolidated repository state into one materialization/publication authority and deleted the predecessor `transaction_action` module, command-local byte planning, duplicate marker composition, profile byte preplanning, and split validation/repair paths.
- Migrated JSON and in-memory behavior to the same captured-image validation and repair semantics, including tolerant malformed-rules reporting and canonical fixture initialization.
- Fixed full-suite discovery and fixture defects without restoring compatibility fallbacks.
- An independent security audit found five blocking transaction defects. The correction now uses copied and synced rollback backups, inode-replacement `SetMode`, pre/post-binding root checks through the commit decision, physical Windows identities, and semantic journal validation that fails closed instead of panicking. Adversarial hard-link, root-swap, rollback, Windows, and malformed-journal regressions cover the fixes.
- Closed feature-only lint drift, one broken contrib documentation link, and a gitleaks prose false positive. The full adopter-facing docs surface passes.
- The source/documentation patch from `656b9a13` through `f8d75181` is 6,769 additions and 6,474 deletions (net +295). The architecture consolidation itself was net -408; the final positive delta is the security/race regression coverage added after review.

## What to do next

1. Start wave 7 only after reading this handoff and the current progress ledger.
2. Amend stale story `a3a788f3` to the invoker-approved direct-main delivery model before completion; do not recreate or treat the retired integration branch as authority.
3. Execute `b1621508` with explicit acceptance for:
   - mode-only repair on both JSON and InMemory backends;
   - mismatched or unresolved profile provenance producing a zero-write result on both backends.
4. Keep `ed3e773c` lead-owned because it edits `.jit/invariants.toml` and the rendered AGENTS projection. Its footprint is otherwise conflict-free with the two implementation children.
5. Preserve the wave-7 independence: no child currently needs to claim or edit another child’s source-of-truth files.

## Traps

- Do not restore any deleted publisher, transaction-action API, marker composer, byte-bag profile planner, root-prefix adapter, or split validation path.
- `JsonFileStorage::new` still has an infallible bootstrap-lock fallback through `root.parent().unwrap_or(".")`. Removing it cleanly needs a fallible or explicit-layout constructor; do not add another inference shim.
- `configure_repository_layout` cannot report rejected rebinding because it returns `()`. Treat this as an API smell, not permission to silently overwrite layout authority.
- `MaterializationRequest` and mutation intent still overlap as taxonomies; output-specific counts/reports, public `assemble_config`, and the repository string formatter are adjacent convenience surfaces that should be reduced only with a concrete consumer migration.
- `PathReadError::OutsideRepoRoot` also represents in-root non-regular targets, so its name overstates the classification.
- Server test `test_get_document_content_not_yet_implemented` is stale and duplicative.
- Ambient path revalidation detects every modeled pre-decision root swap but cannot mathematically exclude an unconstrained rename after the last check without stronger filesystem locking/protocol machinery.
- The GitHub security-audit workflow is pre-existing fail-open (`cargo install ... || true` and `continue-on-error`). Local issue gates are blocking, but release assurance should fix the workflow rather than cite it as security evidence.
- MCP schema tests pass but emit many “Definition not found” and circular-reference warnings. They are noisy enough to hide new generator regressions and deserve a separate cleanup, not suppression inside wave 7.
- Registry-backed `npm audit` was not run because it would disclose lockfile dependency metadata without specific egress authorization. Configured Cargo audit, npm-ci, MCP CI, and secret detection all passed.

## Open questions

- Should wave 8 require specifically authorized npm registry audits for both Node workspaces, or is the configured blocking security surface intentionally Cargo audit plus workspace CI?
- Should release assurance replace ambient root revalidation with a stronger host-specific rename/locking protocol, or document the current last-check boundary as the supported threat model?
- Which later issue should own the fail-open GitHub security workflow and MCP generator-warning cleanup? Neither belongs in the conflict-free wave-7 implementation footprints.

## Reference artefacts

- Plan: `dev/active/cdc840ad-plan.md`
- Progress ledger: `dev/active/cdc840ad-progress.json`
- Consolidation commit: `c5b35092`
- Transaction security commit: `94e1ae60`
- Feature lint correction: `fa3b0549`
- Full-surface docs correction: `92bb653f`
- Secret-scan prose correction: `a7fffe45`
- Frozen candidate/evidence head reviewed by all gates: `f8d75181`
- External code review: run `aee5b723-8284-4ecd-b093-56ce631240b3`, zero findings
- Completed issue: `661d6be2`
- Next wave: `a3a788f3`, `b1621508`, `ed3e773c`

STOP: the user requested handoff after the current issue. Do not claim or begin wave 7 in this session.
