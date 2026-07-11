# Review — Root and component documentation audit (36d5451e) — round 5

**Verdict:** FAIL

## Gate status

- `repo-validate`: passed at the rework commit.
- `docs-mechanical`: passed with the declared root/component footprint.
- `doc-review`: failed, run `5f5c4d9b-57cd-4e97-bbb1-5ac949a87f35`.

## Prior-findings regression table (Tier 1.5)

All findings from the previous four reviews remain closed at HEAD; the reviewer explicitly confirms the prior MCP, Node 20, CLI, and routing findings are addressed. The following are new findings, not regressions.

| Round | Finding | Status at HEAD | Evidence |
|---|---|---|---|
| R5 | Individual API container mounts an uninitialized named volume | open | `INSTALL.md:111`; `jit-server` rejects an uninitialized data directory. |
| R5 | All-in-one image is claimed to serve UI assets despite mismatched nginx and Dockerfile directories | open | `INSTALL.md:152`; `Dockerfile:65`, `docker/nginx.conf:36`. |
| R5 | README says prechecks must pass before all work starts | open | `README.md:138,160`; direct `in_progress` updates do not run prechecks. |

## Success criteria (Tier 2)

- [ ] REQ-01 and REQ-04 — unmet by the three source-contradicting claims above.
- [x] REQ-02, REQ-03, REQ-05, and REQ-06 — the gate reports links/citations, current-tense prose, diagrams, and projection/citation handling clean.

## Stale-narrative sweep (Tier 2.5)

The independent reviewer reports no legacy, migration, or future-facing product narration in the scoped adopter documentation.

## Deferred-items audit (Tier 2.75)

The plan and audit-note follow-up entries remain explicit Group-C non-goals for missing engine/projection surfaces. They do not defer the three current documentation claims, which are in scope and require correction.

## Holistic findings (Tier 3)

The remaining errors are limited to the declared root/component footprint but demonstrate that the container and lifecycle narrative still needs one coherent source-backed correction.

## Required changes

1. Initialize `jit-data` before the individual API container starts, or require mounting an existing initialized repository.
2. Replace the all-in-one UI-serving claim with a source-backed working flow (for example Compose/separate containers), or accurately state its current capability.
3. State that prechecks are executed through `jit issue claim`; retain the unconditional requirement that gates pass before `Done`, without claiming every path into `in_progress` runs prechecks.
