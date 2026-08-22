# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 14

**Date:** 2026-08-13T22:25:00+03:00
**Session number:** 14
**Prior handoffs:** `handoff.md`, `handoff-2.md` … `handoff-13.md` in this directory

## Current state

- Epic: `c639cfb5` — state: in_progress
- Wave in progress: implementation waves 1–23 are done; epic completion is blocked on one owner decision from the final holistic review
- Children summary: all 5 direct children and all 30 progress-plan issues are done; epic is the only in-progress member
- Active claims: epic `c639cfb5` claimed `agent:lead-c639cfb5`; no worker claim remains active
- Open escalations: choose whether the epic contract follows the completed repository-directory architecture or reverses it by reintroducing embedded packages; option A is recommended below
- Progress file: `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Main: documentation remediation merged; Rust, repository-validation, documentation-mechanics, and documentation-review gates pass; final holistic-review run `13149c1f` fails one high finding

## What just happened

- Completed and merged `b5bb4b9c`: removed the retired `ProfileApplied`/`profile_applied` event contract; branch cargo-ci and independent review passed, then merged Rust and MCP suites passed.
- Re-ran `dd268d0f`: repository validation and holistic review passed after the event cutover; story transitioned to done.
- Reconciled every recorded pitfall against all 14 epic criteria; none remains open or merely assigned.
- Ran the first epic gate batch: cargo-ci passed (4,543 tests, 63 doctests, 23.825 s measured suite), repository validation passed, docs mechanics passed, and doc-review failed because local authoring exposed no usable v2 manifest contract.
- Created, reviewed, and merged `08883a21`: `docs/reference/profiles.md` now carries a parser-backed v2 skeleton, every declaration family, exact variable-reference allowlist, and canonical registry links. One lead rework removed four accuracy/duplication defects; issue docs mechanics and independent doc review passed.
- Re-ran epic repository validation, docs mechanics, and doc review: all passed; prior doc finding closed.
- Final holistic-review run `13149c1f` failed one high finding: the epic still promises embedded profiles while production deliberately supports only repository-directory packages and rejects `ProfileOrigin::Embedded` records.
- Traced the contradiction: `ff1bbada` and commit `5324ae4a1` removed compiled packages, compiled origin, and fallback resolution under domain-agnostic/canonical-cutover; c639cfb5 was replanned later but retained obsolete embedded wording, and `9a9a4cfc` reintroduced an unsupported provenance-only enum variant.

## What to do next

- [ ] Obtain the invoker's decision on the embedded-profile contract; do not dispatch or amend issue descriptions before explicit approval.
- [ ] If option A is approved, amend the named embedded assumptions in c639cfb5 (background, REQ-01/06/09/11, D-01/D-02, and any directly stale child criterion such as eac6ec13 REQ-02) to the repository-directory contract, preserving all lifecycle capabilities and package content.
- [ ] Under option A, create one bounded remediation task deleting the provenance-only `ProfileOrigin::Embedded`, `EmbeddedProvenanceUnavailable`, their tests/schema residue, and stale current documentation/issue narrative; add semantic structural protection against reintroduction. Use a high-reasoning implementation worker because it crosses record wire, command resolution, schema, and tests, but do not redesign the resolver.
- [ ] Re-run the remediation's configured gates; merge; run exact integrated Rust and MCP evidence if schema changes.
- [ ] Re-run all five epic gates with prior-finding regression checks. The cumulative table must show the manifest-authoring finding and embedded-contract finding closed at HEAD.
- [ ] On full pass, produce the completion report, transition the epic to done, archive the active artifacts via the configured archival mechanism, link the final report and plan to the epic, install the dogfood binary, and stop.

## Traps — do not repeat these

- **Do not implement the epic's embedded wording literally without owner approval.** `ff1bbada` hard REQ-01/03/05 and commit `5324ae4a1` deliberately removed the compiled package/origin/resolver; restoring it reverses completed shared architecture and violates `@/inv/domain-agnostic`.
- **Do not amend only the epic text and leave `ProfileOrigin::Embedded`.** `commands/profile.rs:2760-2764` proves it is an unsupported provenance token, and holistic-review F1 explicitly binds that residue to REQ-12/canonical cutover. Option A needs both contract correction and clean deletion.
- **Do not argue that the final reviewer misread the criteria.** The issue literally promises embedded support in multiple hard criteria and decisions. Per no-argue discipline, either satisfy it or owner-authorize the contract correction.
- **Do not use the old epic research as current architectural evidence.** `c639cfb5-research.md` describes the pre-extraction embedded runtime; it predates `ff1bbada` and is now historical planning evidence, not the shipped boundary.
- **Do not accept a minimal manifest example as complete authoring documentation.** `08883a21` required the full declaration-family contract and exact contribution templating allowlist; commit `859636f57` is the reviewed form.
- **Do not re-run a marginal cold Rust timing failure without a warm diagnostic sample.** The integrated tree first measured 30.509 s then passed warm at 23.664 s; the authoritative epic cargo gate passed at 23.825 s.
- Prior handoffs' unresolved traps remain in force.

## Open questions needing invoker input

- Question: Should c639cfb5 be corrected to the completed repository-directory package architecture, or should JIT reintroduce embedded `jit-dogfood` resolution?
  - Context: The epic's embedded wording survived replanning after another completed v1.0 container removed all compiled-in profiles under the domain-agnostic clean cut. Final holistic review correctly found the code cannot meet the literal epic contract.
  - Options: A) authorize contract correction to explicitly addressed repository-directory packages and delete the unsupported Embedded provenance residue; B) reintroduce compiled-in package/resolver support across the full lifecycle, reversing `ff1bbada`; C) change text only and retain the dead token (not sufficient).
  - Recommendation: A — it preserves every adopter capability and the north star while keeping one package model/resolver and the already-completed domain-agnostic architecture.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Blocking gate: `.jit/gate-runs/13149c1f-70b1-42a7-b854-2ed310640f9f/result.json`
- Conflicting completed issue: `jit issue show ff1bbada`; removal commit `5324ae4a1`
- Unsupported residue: `crates/jit/src/commands/profile.rs:2760`, `crates/jit/src/domain/types.rs:1115`
- Manifest docs remediation: `jit issue show 08883a21`; reviewed docs commit `859636f57`
- Planning/history: `plan.md`, `breakdown.json`, `progress.json`, `investigation.md`, `c639cfb5-research.md`
- Charter: `dev/vision/9db27a3a-charter.md` D-8 and D-14
