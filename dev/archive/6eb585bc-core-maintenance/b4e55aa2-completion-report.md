# Story complete: Ground code review in AGENTS.md and addressable policy (b4e55aa2)

**Started:** 2026-07-12
**Completed:** 2026-07-13
**Assignee:** agent:jit-execution-lead

## Summary

Code review now discovers canonical engineering prose from applicable `AGENTS.md` files, resolves bounded addressable policy from configured sources of truth, and records governing qualified IDs in structured findings. The generic findings model remains policy-agnostic, the shared review wrappers remain byte-identical and tool-agnostic, and a recorded positive Codex review demonstrates the integrated contract.

## Metrics

| Metric | Value |
|---|---:|
| Children completed | 4 / 4 |
| Waves executed | 3 |
| Rework cycles | 4 |
| Escalations | 1, resolved |
| Sub-agent dispatches | 9, including reworks and the user-directed clarification |
| Issues created during execution | 0 |

## Success criteria

- [x] REQ-01 — applicable `AGENTS.md` files form the complete root-to-path prose baseline, with specialization and contradiction handling — delivered by `8b479a72`.
- [x] REQ-02 — the design maps the complete historical rubric to exactly one allowed, specific canonical owner per principle — delivered by the story design and cumulative story rework in `c64fb205` and `12c77ee7`.
- [x] REQ-03 — advisory `@/inv/pid-safety` is registry-owned, projected, and required as a governing finding reference — delivered by `440d190f` and `8b479a72`.
- [x] REQ-04 — canonical public-API documentation and contextual test-coverage prose live in root `AGENTS.md` — delivered by `440d190f`.
- [x] REQ-05 — qualified-item discovery is bounded, resolved through configured sources, and confined to the attributable impact cone — delivered by `8b479a72`.
- [x] REQ-06 — relationship labels are evidence claims, with attributable unsupported claims blocking and unrelated pre-existing defects advisory — delivered by `8b479a72`.
- [x] REQ-07 — every code review emits the fixed five-field evidence header and explicit `none` values — delivered by `8b479a72` and demonstrated by `155a2d43`.
- [x] REQ-08 — `GateFinding.references` is optional/default-empty across parser, schema, storage, output, and legacy records while remaining opaque — delivered by `99604a20`.
- [x] REQ-09 — governed findings carry valid qualified references while ordinary correctness findings may omit them — delivered by `99604a20` and `8b479a72`.
- [x] REQ-10 — executable validation remains owned by `@/gate/jit-validate`; review consumes latest evidence without rerunning passing gates or treating incomplete peer judgment as a defect — delivered by `8b479a72`.
- [x] REQ-11 — deterministic positive and negative tests cover policy discovery, precedence, source ownership, relationship claims, headers, references, legacy compatibility, wrapper parity, and rubric non-duplication — delivered by `99604a20` and `8b479a72`.
- [x] REQ-12 — user and contributor documentation explains policy ownership, addressable traceability, structured references, and the repository-prompt/tool-agnostic-wrapper boundary — delivered by `440d190f` and `8b479a72`.
- [x] REQ-13 — a representative live review records run/session identity, measurements, applicable policy, resolved items, gate evidence, truncation handling, references, and verdict — delivered by `155a2d43` in `dev/active/b4e55aa2-code-review-live-verification.md`.

## Wave execution log

**Wave 1:** `99604a20` and `440d190f` ran concurrently over disjoint surfaces. They established the backward-compatible structured-reference contract and canonical policy sources without changing reviewer behavior.

**Wave 2:** `8b479a72` landed the prompt, both wrappers, regression tests, and affected documentation atomically. Rework corrected transport ownership, and an invoker clarification made peer-review status evidence non-blocking by itself.

**Wave 3:** `155a2d43` recorded and linked the representative positive live-review report from the completed cutover.

**Container review:** Aggregate review drove two cumulative design-matrix reworks: first replacing compound and broad owners with singular owners, then covering every historical rubric principle with exactly one allowed owner. The final story review passed with zero findings.

## Key decisions

- Kept the two foundations independently shippable and prohibited reviewer-prompt changes until their completion.
- Kept reference values opaque in the generic engine; repository-specific resolution remains reviewer procedure.
- Kept `scripts/ai-review.sh` and `contrib/gates/ai-review.sh` byte-identical and free of reviewer-tool policy.
- Assigned canonical prose to applicable `AGENTS.md`, registry facts to exact qualified items, and review behavior to named reviewer-procedure rules.
- Recorded every gate status as evidence while ensuring incomplete peer review or judgment is not categorically blocking; executable CI or validation evidence blocks only when tied to an attributable failure or materially unverified hard criterion.
- Used deterministic negative tests instead of a fragile live negative fixture.

## Escalations

- The execution environment initially classified the configured Codex reviewer as an untrusted external disclosure. The invoker confirmed that Codex is trusted and granted standing authorization for `jit gate evaluate`; all subsequent configured review gates ran normally.

## Issues discovered during execution

No additional JIT issues were created. A host crash left one stale claims lock, which `jit recover` removed without data loss. Review found and closed an ambiguous human rendering of reference arrays, inaccurate wrapper/JIT parsing ownership prose, and incomplete policy-owner mappings within the existing story scope.

## Holistic quality notes

- Every child and the story passed all configured gates.
- The final aggregate code review passed with zero findings after verifying both prior story-level findings cumulatively.
- The prompt is procedural rather than a replacement engineering rubric.
- The ownership matrix contains 41 singular, specific owners and explicitly treats enforcement, citations, projections, schema, storage, transport, and issue inputs as support rather than co-ownership.
- The representative live run shows a consistent five-field header, structured zero-finding payload, terminal verdict, process exit, and stored status.
