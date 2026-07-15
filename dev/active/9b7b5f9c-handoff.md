# Handoff — Versioned repository profiles and portable JIT dogfood setup (9b7b5f9c) — session 1

**Date:** 2026-07-15T16:36:15+03:00
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `9b7b5f9c` — state: backlog (assigned to `agent:jit-execution-lead`)
- Wave in progress: wave 1 of 9
- Children summary: 0 done, 3 in_progress, 9 backlog/ready, 0 rejected
- Active claims: `866a5bbd`, `92039d9c`, and `be542b98` are assigned to `agent:worker` since this session.
- Open escalations: cross-epic dependency/audit remediation; pre-existing secret-scan findings outside the epic; informed consent for AI review data export.
- Progress file: `dev/active/9b7b5f9c-progress.json` (reflects the above)

## What just happened

- Recovered one stale `.git/jit/locks/claims.lock`; repository validation was green before dispatch.
- Confirmed the approved planning bracket is complete: planning `ca024a2b` and breakdown `96326159` are done with their gates passed; 12 implementation tasks cover all epic criteria.
- Persisted a nine-wave plan; wave 3 is explicitly blocked by cross-epic issue `d3709cc6`.
- Dispatched wave 1 in SHA-anchored worktrees and passed the authorized `security-review` precheck for `866a5bbd`.
- Integrated transaction kernel `866a5bbd` (`92a4ad1e` plus merge), portable checks `92039d9c` (`822ea0b4` plus merge), and overlay validation `be542b98` (`5bad31d0` plus merge).
- Verified every merge commit independently with `scripts/verify-commit-builds.sh`; all three built from archived commit sources.
- Reworked overlay validation twice: `fad3e5a0` restored exit-1 rule reports versus exit-4 structural errors; `3447c301` restored legacy structural diagnostics. Exact regressions and 11 repository-view tests pass.
- Cargo CI for `866a5bbd` passed after clearing stale generated incremental state: 3,349 tests passed, zero failed on the preceding full run; the recorded clean retry passed.
- `repo-validate` and `security-review` pass for `866a5bbd`.
- `secret-detection` fails only on two redacted, pre-existing `generic-api-key` matches in `.jit/issues/0c3fcf3a-7d9d-4c3e-a40b-4db8d42b6bbe.json:4` and `dev/active/8b05a612-plan.md:166`; neither belongs to this epic.
- `dependency-audit` fails with 10 vulnerabilities and 7 denied warnings. Existing issue `d3709cc6` owns remediation and depends on backlog story `73482aa1` outside this epic.
- `code-review` was not run: the execution environment requires informed consent before exporting issue context and attributable patches to the external Codex reviewer.

## What to do next

- [ ] Resolve the three invoker questions below and record the answers in `dev/active/9b7b5f9c-progress.json`.
- [ ] If approved, make only the two narrow secret-like prose edits, rebuild/install JIT with current HEAD provenance, and rerun `secret-detection` for `866a5bbd`.
- [ ] Wait for or explicitly coordinate completion of `73482aa1` → `d3709cc6`; then rerun `dependency-audit` for `866a5bbd`.
- [ ] With informed consent, run `code-review` for `866a5bbd`; complete the full six-tier lead review and rework any gate findings.
- [ ] Evaluate all gates for `92039d9c` and `be542b98`; AI `code-review`/`doc-review` require the same informed consent.
- [ ] After every wave-1 gate passes, mark the three issues done, update progress to wave 2, commit JIT state separately, and dispatch `eceffc17`.

## Traps — do not repeat these

- **Do not run `jit gate evaluate-all` without informed external-review consent.** The environment rejected it because `code-review` exports issue context and attributable patches through `codex exec`; run deterministic gates individually until consent exists.
- **Do not collapse repository-view semantic findings into `anyhow` errors.** That changed rule findings from exit 1 to exit 4 and broke integrity diagnostics. The accepted split is `RepositoryValidationReport.rule_report` for semantic findings and typed structural errors for exit 4 (`fad3e5a0`, `3447c301`).
- **Do not infer that Git-free document validation is new or broken.** The pre-overlay `CommandExecutor::validate_document_references` intentionally skipped document checks when `git2::Repository::open` failed; the final view implementation preserves that existing behavior.
- **Do not rerun `cargo-ci` after targeted dev-profile tests without clearing generated incremental state.** The gate intentionally fails on non-empty `target/debug/incremental`; `cargo clean` cleared the cache-only failure before the passing retry.
- **Do not treat dependency-audit failures as transaction-kernel rework.** The advisories are repository-wide and owned by cross-epic issue `d3709cc6`, which itself depends on `73482aa1`.
- **Do not weaken or bypass secret detection.** Its two findings are redacted and pre-existing, but the gate remains failed until the source prose is changed with explicit cross-issue scope approval.
- **Reinstall JIT after every commit before evaluating a gate.** The stale-binary guard compares installed build provenance with current HEAD, including JIT-state commits.
- **Worktree workers can exhaust `/tmp` quota.** A completed worker left `/tmp/jit-target-92039`; `cargo clean --target-dir /tmp/jit-target-92039` reclaimed the generated cache. Inspect and clean only task-owned generated targets.

## Open questions needing invoker input

- Question: Do you explicitly consent to sending issue descriptions, linked-document context, and attributable `jit:<short-id>` code patches to the external Codex reviewer for `code-review` and `doc-review` gates?
  - Context: Blanket gate authorization did not satisfy the environment's informed workspace-data-export requirement.
  - Options: consent to the described export; decline and leave AI gates pending.
  - Recommendation: Consent if repository code may be processed by the configured external reviewer; gates cannot otherwise pass.
- Question: May the lead edit the two pre-existing secret-like prose occurrences outside epic `9b7b5f9c`?
  - Context: Redacted gitleaks findings are in production-readiness issue `0c3fcf3a` and plan `dev/active/8b05a612-plan.md`, not in wave-1 code.
  - Options: authorize narrowly replacing only the flagged secret-like token in both sources; have the owning epic fix them externally; stop this epic with `secret-detection` failed.
  - Recommendation: Authorize the two narrow wording edits, with no scope or behavioral change.
- Question: How should the existing cross-epic chain `73482aa1` → `d3709cc6` be completed?
  - Context: `dependency-audit` cannot pass and `0aec3b1e` cannot become ready until that production/core-maintenance work lands.
  - Options: run a separate execution lead for the owning containers and resume this epic afterward; explicitly override the single-epic invariant and expand this execution; stop with the dependency unresolved.
  - Recommendation: Complete the chain through its owning execution lead, then resume `9b7b5f9c`; this preserves DAG ownership and single-epic scope.

## Reference artefacts

- Epic: `jit issue show 9b7b5f9c`
- Design docs: `dev/active/9b7b5f9c-mvp-scope-brief.md`
- Planning docs: `dev/active/9b7b5f9c-plan.md`
- Progress: `dev/active/9b7b5f9c-progress.json`
- Gate evidence: `.jit/gate-runs/` entries linked from `jit gate status 866a5bbd --all --json`
- External blockers: `jit issue show d3709cc6`; `jit issue show 73482aa1`
