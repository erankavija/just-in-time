# Handoff — Version 1.0 release (`b80e3c70`) — session 4

**Date:** 2026-07-30

**Session number:** 4

**Prior handoffs:** `dev/active/b80e3c70-v1-release/handoff-3.md` and `dev/active/b80e3c70-v1-release/handoff.md`

**Main baseline before this handoff:** `25657567`

**Progress authority:** `dev/active/b80e3c70-v1-release/progress.json`

## Current state

- Epic `b80e3c70` remains assigned to `agent:jit-execution-lead`; its release sink remains intentionally unmet.
- Wave 2a is complete: `7b29019c`, `5e538f48`, and `3b033738` are Done with every configured gate passed.
- Subwave 2b is next but undispatched: `e0fed1bb`, **Make the normal validation suites reusable**.
- Subwave 2c remains serialized behind 2b: `f7f80d53`, **Enforce reusable blocking dependency audits**. Both issues edit the workflow-contract authority.
- Wave-3 issue `3a3b5704`, **Remove obsolete Docker topologies and migrate every active consumer**, became Ready when `3b033738` completed. Do not pull it into wave 2; it owns the later Compose, split-image, script, and documentation cutover.
- Main is clean and `jit validate` passes.

## Wave 2a outcomes

- `5e538f48` aligned the CLI, server, MCP package, and web package at `1.0.0`; added a manifest-derived release/tag/lock/compatibility check; removed the stale nested Cargo lock; and established one canonical compatibility page distinct from `.jit` schema versioning.
  - Independent review required exactly one compatibility marker and removal of stale `v0.1.0` tag guidance. Rework passed.
  - Merge: `7574ebfa`; completion evidence: `d0f458d0`.
  - Gates: cargo-ci, npm-ci, mcp-ci, repo-validate, code-review, and jit-validate passed.
- `7b29019c` corrected archive destination derivation to strip an exact repeated owner only after an exact configured `issue_scoped_areas` prefix, including nested areas. Flat managed areas and deeper vendored repetition remain unchanged.
  - All 54 exact full-slug offenders moved byte-identically. The lead migrated 34 live document paths and 3 structured asset paths; historical descriptions and immutable event history were not rewritten.
  - Independent review caught and rework fixed the initial one-component-area assumption. The first configured review then caught an invalid `satisfies:REQ-13` label; the label was removed and review passed.
  - Merge: `6ade0d31`; reference migration: `0361b223`; completion evidence: `9e494c8c`.
  - Gates: cargo-ci, code-review, and docs-mechanical passed.
- `3b033738` replaced the legacy root image with one non-root PID-1 `jit-server` image serving the embedded web UI against a repository mounted at `/repo`.
  - Runtime tests cover default `10001:10001` and host-mapped identities, mount preflight, no auto-init, issue and linked-document search from `/repo`, host-visible `.jit` writes, health, three SSE connections, one stalled ordinary connection, five-second application force-close, and clean exit before the ten-second runtime deadline.
  - Independent review found fail-open engine selection, shallow shutdown-log assertions, and missing SPA deep-link fallback. Rework closed the first two. The invoker authorized a narrow server-layer expansion; embedded fallback now serves safe extensionless client routes from uncached `index.html` while unknown API, missing file-like assets, invalid encodings, and traversal-shaped routes remain 404.
  - Final cumulative review passed. Full rebuilt Podman runtime passed in both identity modes; Docker was unavailable.
  - Merge: `f93a30c4`; scope-boundary evidence: `3c8db804`; completion evidence: `25657567`.
  - Gates: cargo-ci, npm-ci, mcp-ci, repo-validate, code-review, and jit-validate passed.

## Surfaced path-format risk

- The exact full-slug defect owned by `7b29019c` is closed: zero `dev/archive/<container>/<area>/<same-container>/...` paths remain.
- A distinct bare-short-ID family remains deliberately untouched: 46 paths total, 25 under current `presentations` and 21 under legacy `features`, such as `dev/archive/1cc809de-repository-state-quality/presentations/1cc809de/...`.
- Do not silently treat a short-ID prefix as equivalent to a full container slug. That requires separate planning because prefix matching can create false owner classifications.
- `vendor/reveal.js/reveal.js` is legitimate third-party structure and must remain allowed.

## Rejected issue

- `a122b9b3`, **Enforce semantic types, canonical cutovers, and architecture dependency direction**, is Rejected with `resolution:ill-planned`.
- Its exploratory branch remains unmerged for forensic preservation. Do not resume or merge it.
- The rejection reflects an overbroad enforcement promise, not permission to weaken the underlying project invariants.

## What to do next

1. Re-read `e0fed1bb` and current `.github/workflow-contract.yml` after the landed version-contract changes.
2. Confirm main is clean, claim `e0fed1bb`, create a fresh SHA-anchored worktree, and dispatch one task-fit worker. This issue owns `ci.yml` and reusable normal-suite workflow contracts.
3. Independently review, merge, run exact-commit verification, reinstall through `scripts/install-jit.sh`, evaluate gates sequentially, and commit JIT completion state separately.
4. Dispatch `f7f80d53` only after `e0fed1bb` is Done. It shares workflow-contract surfaces and additionally owns security-audit and release-workflow audit predecessors.
5. After wave 2 is complete, proceed to wave 3 as recorded in `progress.json`; do not absorb `3a3b5704` into image issue `3b033738`.

## Traps — do not repeat these

- Workers do not edit `.jit`, claims, issue states, or configured gates. Those remain lead-owned.
- Keep implementation commits separate from JIT gate/state commits.
- Run configured gates strictly sequentially from a clean main checkout.
- Keep the incremental compilation preflight first in cargo-ci. The gate is normally silent for several minutes.
- Install the dogfood binary only through `scripts/install-jit.sh` and run `scripts/verify-commit-builds.sh` after every implementation merge.
- A sandboxed cargo-ci attempt can fail before checks because its configured temp root under `~/.cache` is read-only. Preserve that run and rerun with authorized cache access; do not misclassify it as a code failure.
- Do not dismiss graceful-shutdown timing failures without comparison evidence. During wave 2a the identical changed-tree binary both failed in 0.30 seconds and passed in 5.35 seconds; an alternating changed/base/changed sequence passed.
- Do not let issue-impact review collapse planned successor work into its prerequisite. `3b033738` proves the replacement image; dependent `3a3b5704` owns removal and migration of legacy deployment surfaces.
- Continue scanning for repeated development depth and owner identity in paths. Surface suspicious variants rather than normalizing them under an unreviewed equivalence rule.

## Reference artefacts

- Epic: `jit issue show b80e3c70`
- Next issue: `jit issue show e0fed1bb`
- Serialized audit issue: `jit issue show f7f80d53`
- Later topology cutover: `jit issue show 3a3b5704`
- Progress: `dev/active/b80e3c70-v1-release/progress.json`
- Prior handoff: `dev/active/b80e3c70-v1-release/handoff-3.md`
