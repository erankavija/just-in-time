## Epic Complete: Complete profile lifecycle, composition, and upgrades (c639cfb5)

**Started:** 2026-08-09T04:36:58+03:00  
**Completed:** 2026-08-13T23:33:26+03:00  
**Assignee:** agent:lead-c639cfb5

### Summary

JIT now ships a complete offline profile lifecycle over repository-directory packages: adopters can author or capture packages, select and compose them deterministically, inspect or rehearse changes, reconfigure and upgrade with drift-safe ownership checks, and exchange verified archives. The lifecycle uses one canonical package graph and recoverable publication path, works without Git, exposes consistent human/JSON/schema/MCP contracts, and documents the complete v2 manifest and operational workflow.

### Metrics

| Metric | Value |
|---|---|
| Direct children completed | 6 / 6 |
| Delivered descendants reviewed | 33 |
| Waves executed | 24 |
| Rework cycles | 37 across all recorded issues |
| Escalations and owner decisions | 12, all resolved or mitigated |
| Sub-agent dispatches | At least 68 recorded: 31 wave entries plus 37 rework cycles; auxiliary reviews were not counted separately |
| Issues created during execution | 9 |

### Success Criteria

- [x] REQ-01 — one versioned offline directory-package model and repeatable ordered selectors across the lifecycle — delivered by `f2ef652a`, `eac6ec13`, and `8bbe6557`.
- [x] REQ-02 — deterministic dependency, compatibility, and contribution-graph resolution with pre-publication rejection — delivered by `474a90a8`.
- [x] REQ-03 — semantic composition, identical-definition deduplication, shared ownership, and conflict rejection — delivered by `cbcd9318`.
- [x] REQ-04 — deterministic explicitly non-secret variables and declared rendering references — delivered by `fc47a7bf` and guarded across every public surface by `3dcce8a8`.
- [x] REQ-05 — durable directory origin, package identity, resolved inputs, and semantic/file ownership without a shadow configuration authority — delivered by `9a9a4cfc` and completed by `8bbe6557`.
- [x] REQ-06 — coherent list/show/validate/diff/capture/pack/add/apply/reconfigure/upgrade presentation, dry runs, schema, and audit output — delivered by `7156f64b`, `f2389b18`, `96268a98`, `9f493686`, and `cb26de35`.
- [x] REQ-07 — three-way drift decisions for updates, conflicts, shared ownership, and safe removal during upgrade — delivered by `44ec7192` and `7156f64b`.
- [x] REQ-08 — aggregate, atomic, recoverable, idempotent, Git-free publication — delivered by `9fad8581`, `6574f951`, `d7558ef6`, and `8021d507`.
- [x] REQ-09 — the distributed repository-local `jit-dogfood` package remains valid without duplicated workflow inventory — delivered by `f2ef652a`, the pre-release record cutover `9a81ab12`, and directory-source closure `8bbe6557`.
- [x] REQ-10 — one canonical adopter guide for authoring, capture, exchange, composition, ownership, drift, and upgrades — delivered by `b3d92595` and manifest-contract repair `08883a21`.
- [x] REQ-11 — automated lifecycle, interruption, concurrency, Git-free, schema, audit, archive, and acceptance coverage — delivered across `acf49914`, `6574f951`, `3dcce8a8`, `d7558ef6`, `8021d507`, `b5bb4b9c`, and `8bbe6557`.
- [x] REQ-12 — one canonical resolver/composer/ownership/publication design with superseded paths removed in the same cutover — delivered by `9a81ab12`, `3dcce8a8`, `b5bb4b9c`, and `8bbe6557`.
- [x] REQ-13 — whole-tree capture and refresh from declared live repository targets, including owned semantic contributions — delivered by `6a479c58`, `5a3ecba0`, and end-to-end regression `acf49914`.
- [x] REQ-14 — bounded, digest-carrying offline pack/add with unsafe or malformed archive rejection — delivered by `15cc28c5`, publication-bound repair `cf42d08b`, and checkpoint `f6a17e0d`.

### Wave Execution Log

**Wave 1:** Canonicalized both supported manifest versions into one model.  
**Wave 2:** Replaced single-profile arguments with ordered repeatable selectors.  
**Wave 3:** Added deterministic dependency, compatibility, and conflict graph resolution.  
**Wave 4:** Closed the package-graph checkpoint and added non-secret variable resolution.  
**Wave 5:** Added semantic composition and shared ownership.  
**Wave 6:** Persisted durable package, input, target, and contribution ownership claims.  
**Wave 7:** Published complete selections through one recoverable transaction.  
**Wave 8:** Added three-way target decisions and one aggregate lifecycle event.  
**Wave 9:** Exposed reconfigure and upgrade, including dry-run conflict decisions.  
**Wave 10:** Added safe whole-tree capture from declared repository targets.  
**Wave 11:** Removed the pre-release record migration boundary.  
**Wave 12:** Added verifiable package archives and closed the ownership story checkpoint.  
**Wave 13:** Removed the package-tree path-count publication mismatch.  
**Wave 14:** Added applied-package and repository agreement checks.  
**Wave 15:** Unified rehearsal and execution planning decisions.  
**Wave 16:** Refreshed owned semantic contributions from repository state.  
**Wave 17:** Unified presentation envelopes and added the full edit-capture-reapply acceptance journey.  
**Wave 18:** Closed the authoring and offline-exchange story checkpoint.  
**Wave 19:** Completed adopter documentation, interruption recovery evidence, and structural cutover guards.  
**Wave 20:** Proved concurrent callers serialize at the canonical mutation lock.  
**Wave 21:** Proved the complete lifecycle in a repository that never has `.git`.  
**Wave 22:** Removed the superseded per-package `profile_applied` event and closed the surface checkpoint.  
**Wave 23:** Added the parser-backed v2 manifest authoring contract and exact reference allowlist.  
**Wave 24:** Applied owner-approved Option A: corrected the contract to repository-directory packages and removed the unsupported embedded-provenance wire/error/bypass.

### Key Decisions

- Kept the lifecycle on one repository-directory package source. `jit-dogfood` is distributed package content, not executable behavior; embedded provenance without a resolver was deleted.
- Treated capture as the single authoring and drift-recovery operation. Repository-authored live assets and owned semantic contributions refresh; generated regions and install-only assets remain package-authored.
- Kept composition semantic and order-independent: identical definitions share ownership and differing definitions fail instead of selecting a winner.
- Persisted resolved non-secret values so reconfiguration is deterministic, while rejecting any secret or sensitive input channel across manifest, CLI, schema, MCP, audit, and structural guards.
- Kept archive exchange offline and integrity-focused. The digest detects corruption or modification; authenticity remains the adopter's chosen transport channel.
- Removed arbitrary named-payload byte ceilings after valid adopter inputs and template-expanded targets disproved them; discovery remains bounded by one listing, declared membership, and construction-derived shape.

### Escalations

- Shared validation infrastructure (`45cd8529`) was mitigated before direct-main integration.
- `eac6ec13` received an authorized targeted retry and later handoff repair for the dry-run selector contract.
- `fc47a7bf` received one authorized narrow stale-prose retry after its normal rework limit.
- `cbcd9318` received one authorized ownership-convergence retry spanning apply and initialization.
- For `cf42d08b`, the owner chose removal of the per-path budget; the implementation retained depth, listing, and byte bounds and closed every path source.
- The owner clarified refresh coverage by content authority: repository-authored classes refresh; package-authored regions and install-only assets do not.
- Current Rust build-budget constants were folded into `0708d692` for fresh derivation; the accepted 30-second suite budget retained its measured cold margin.
- `5a3ecba0` was scoped to declarations claimed by the applied record rather than every manifest declaration.
- After repeated valid oversized-payload failures, the owner chose shape-bounded, byte-unbounded named agreement reads.
- Final holistic review exposed the embedded-profile contract conflict; the owner approved Option A, and `8bbe6557` removed the dead representation rather than reversing the domain-agnostic package extraction.

### Issues Discovered During Execution

- `08883a21` — document the profile manifest authoring contract; created to close the final documentation-review finding.
- `b5bb4b9c` — remove the superseded `profile_applied` event contract; created from a surface-story holistic finding.
- `cf42d08b` — repair a deep package that packed but could not publish.
- `acf49914` — prove the complete in-place edit, capture refresh, and agreement journey.
- `5a3ecba0` — refresh repository-authored semantic contributions during capture.
- `8bbe6557` — remove unsupported embedded profile provenance after the owner selected directory-only packages.
- `5f71a48d` — reject-reason validation defect, tracked outside this epic.
- `0708d692` — build-gate cache-state defect, tracked under the v1.0 release container.
- `7793b8b7` — obsolete conversion-counter task, rejected after the clean record cutover removed the subject.

### Holistic Quality Notes

- Final forced `cargo-ci` passed with 4,540 tests, 63 doctests, zero warnings, and a 23,610 ms suite clock; every integration-target, executable-size, dependency-feature, profile, and incremental-state budget passed.
- Exact-binary MCP validation passed 65 unit and 14 integration tests. Repository validation and documentation mechanics passed.
- Final documentation review inspected all 33 delivered descendants and found the authoring, composition, capture, exchange, ownership, recovery, reconfiguration, and upgrade surface accurate and discoverable.
- Final holistic review closed the prior embedded-provenance finding and reported: “All hard criteria are verifiably met; no blocking incoherence found.”
- Structural evidence now guards the canonical manifest decoder/model/applied-record authority, aggregate event, non-secret input contract, directory-only profile source, compiled-package absence, and single graph route. Interruption, concurrency, and Git-free journeys exercise the public lifecycle rather than test-only publication alternatives.
- The only adopter's additional feedback is tracked separately in customer epic `4a559332`; it did not expand this epic.
