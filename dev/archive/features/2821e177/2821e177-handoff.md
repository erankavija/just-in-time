# Handoff — Addressing v2: uniform path-based addresses; rule/gate as addressable items (2821e177) — session 1

**Date:** 2026-07-03
**Session number:** 1
**Prior handoffs:** None

## Current state

- Epic: `2821e177` — state: backlog (assigned `agent:jit-execution-lead`; cannot go in_progress while children open)
- Wave in progress: session scope COMPLETE — invoked "until story 0efbc594 completion", which is done; waves 1–8 of the session plan all finished
- Children summary (epic subtree): bracket P `0b611ccf` + B `923e42fb` done; stories `37506c12` + `0efbc594` done with all 7 tasks (`d6c3bff6`, `5222aa47`, `0910a381`, `f4b5e3f4`, `0597febe`, `542f8b6d`, `7a2bbe4f`) done. Remaining: stories `71ebd1e8` (rule kind), `bb7d57a2` (gate kind), `7f22d6cf` (re-addressing migration, scope extended with REQ-04), `9a7106ae` (multi-jit), tasks `d30695e4`, `10db7a00`, `ebb6ad45`, `3d9e9222`, `a1b6b3da`, `a8889340`, plus rule/gate-story subtasks (`cdc33a0f`, `87346f52`, `ae9dd95a`, `42898915`, `f5d35048`, `943abd0b`, `c01f66dd`, `637764ef`, `0910a381`-siblings) and enhancement `cd1da611` — all backlog/ready
- Active claims: epic `2821e177` assigned to `agent:jit-execution-lead` (this lead role); all completed issues released by done-transition
- Open escalations: none (one raised and RESOLVED this session, see below)
- Progress file: `dev/active/2821e177-progress.json` (reflects the above; waves are the SESSION plan toward 0efbc594, not the full epic)

## What just happened

- Dispatched `d6c3bff6` (address parser, sonnet): PASS first review. Worker deliberately added a separate `AddressScope` type instead of extending `Scope` (exhaustive matches at unmodifiable call sites) — sound, accepted.
- Dispatched `5222aa47` (per-(scope,kind) uniqueness, sonnet): PASS after 2 rework rounds, both for stale kind-blind doc comments the reviewers kept finding (regex sweeps missed differently-phrased instances; round 2 used an exhaustive manual read of every doc block — that's the method that worked).
- Dispatched `0910a381` (sugar expansion, sonnet): PASS first review.
- Dispatched `f4b5e3f4` (resolver rewire, opus): surfaced that plan D3's premise is FALSE (see Traps). Escalated to invoker; both recommendations approved → plan amendments **D3-A** (item-backed sugar fallback) and **D11-A** (legacy kindless resolution removed in 7f22d6cf, not now) appended to plan doc §Amendments; `7f22d6cf` description gained REQ-04; `f4b5e3f4` REQ-01 wording aligned. PASS on re-review.
- Closed story `37506c12` after 1 rework (colon rejection in parser + a real masking bug in the legacy fallback found during that rework) + description gained ownership-split plan context so the reviewer stops flagging the label-regex gap (0efbc594's scope).
- Dispatched `0597febe` (label-grammar widening, sonnet): its review flagged the planned labels.rs/defaults.rs divergence window → landed `542f8b6d` (lockstep, sonnet) immediately, then re-reviewed `0597febe` on the consistent tree: both PASS. Closed in dep order.
- Dispatched `7a2bbe4f` (re-split audit, sonnet; rework on opus): 1 rework — `label_credits_id` now routes @-forms through the structural parser (kind parsed but IGNORED for crediting, per D10), named-project errors carry (project, kind, self-id) components. PASS round 2.
- Closed story `0efbc594`: all three gates (cargo-ci, repo-validate, code-review) passed; Tier 2.5/2.75 sweeps clean. **Session target reached.**

## What to do next

- [ ] Next wave per plan §3: stories `71ebd1e8` (rule as addressable kind) and `bb7d57a2` (gate kind over gates.toml) — DAG-independent but both touch `[item_kinds]` config and storage plumbing; plan's execution note says serialize or isolate (this session ran everything serialized in the main tree; that worked well).
- [ ] Then Group C/D: `d30695e4` (enforces links), `10db7a00` (rebind enforced-by), `ebb6ad45` (reference projection), story `7f22d6cf` (migration — REMEMBER its new REQ-04: remove the legacy kindless resolution path in `commands/item.rs` `resolve_item_address` in the same change that re-addresses minted qualified ids).
- [ ] Group E: `3d9e9222` (project-name config), `a1b6b3da` (named-project local-or-error resolution — the `resolve_item_address` NamedProject arm has an honest hook comment), `a8889340` (CLI help text), story `9a7106ae`.
- [ ] Epic close: run epic gates (cargo-ci, code-review, repo-validate), completion report per Section 10.

## Traps — do not repeat these

- **Do NOT treat plan D3's "distinct id-patterns" rationale as true.** The live `requirement` id-pattern is the catch-all `[A-Z][A-Z0-9]*-[0-9]+` (`.jit/config.toml`), matching every `D-NN`/`RISK-NN`; live criteria use TEST-01/SCFA-01/TSTB-01 shapes so it cannot be narrowed. Sugar semantics are governed by amendment **D3-A** (plan doc §Amendments): pattern-ambiguous sugar defers to indexed items at resolution call sites; the pure `expand_sugar_address` keeps its strict `SugarKindAmbiguous` error. Do not "fix" either half to match old D3.
- **Do NOT remove the legacy kind-agnostic `@/<self-id>` path in `resolve_item_address` before story 7f22d6cf.** Amendment D11-A: `derive_scope_items` still mints kindless qualified-id strings that `jit item list` prints; resolution and minting must flip together in 7f22d6cf (its REQ-04). Removing it earlier re-breaks list/show consistency.
- **Do NOT add kind-filtering to `label_credits_id` crediting.** D10 + f4b5e3f4 REQ-04 pin scope+self-id-only credit semantics with tests that must pass UNMODIFIED. The @-form path routes through `parse_kind_segmented_address` but ignores the kind component (see its doc comment). Two review rounds established this exact balance.
- **Do NOT rely on regex sweeps to close "stale doc comment" review findings.** 5222aa47 lost a full rework round because the same stale claim exists in differently-phrased forms ("PER SCOPE", "across any two kinds"). The working method: read EVERY `///`/`//!` block in the touched files and judge each claim, then paste an inspected-locations table (OK/FIXED per row) in the report.
- **Do NOT dispatch a task whose review will see a planned mid-story divergence without landing the closing task first.** 0597febe/542f8b6d: the reviewer (correctly) fails on labels.rs vs CANONICAL_LABEL_REGEX divergence even though the split was planned. Land the pair back-to-back, then review both on the consistent tree (assign-without-transition works for the blocked second task: `jit issue assign`, transition after the first closes).
- **Do NOT expect `jit issue claim` to work on issues with open deps or on the epic container.** Claim auto-transitions to in_progress and exits 4 when blocked; use `jit issue assign` for bookkeeping-only assignment.
- **Reviewers see ONLY the issue description, not the plan.** Three review rounds this session were spent on findings that were plan-sequencing facts (ownership split, dual-source window, D3-A). When a criterion's meaning depends on plan context or an amendment, propagate that context INTO the issue description before running code-review (as done for 37506c12, f4b5e3f4). For the remaining stories: 71ebd1e8/bb7d57a2 descriptions already embed their contracts, but check for cross-story boundaries (e.g. both touch `[item_kinds]`) before dispatch.

## Open questions needing invoker input

None. (This session's escalation — D3 false premise + D11 timing — was resolved: invoker approved item-backed sugar fallback and deferred legacy-path removal to 7f22d6cf.)

## Reference artefacts

- Epic: `jit issue show 2821e177`
- Plan (with §Amendments D3-A/D11-A): `dev/active/2821e177-2d2d-4b25-b64a-4f38723e7fe6-plan.md`
- Design brief: `dev/studies/addressing-v2-rule-gate-items.md`; investigation: `dev/active/2821e177-investigation.md`
- Breakdown specs: `dev/active/37506c12-breakdown-spec.md`, `dev/active/0efbc594-breakdown-spec.md`, `dev/active/71ebd1e8-breakdown-spec.md`, `dev/active/bb7d57a2-breakdown-spec.md`, `dev/active/7f22d6cf-breakdown-spec.md`, `dev/active/9a7106ae-breakdown-spec.md`
- Progress file: `dev/active/2821e177-progress.json`
- Key landed commits: 8e06e5f4 (parser), dcb09a64+38ff51eb+d7714d8b (uniqueness), 6749e0e3 (sugar), 73fe245d+4d1a3f0c (resolution rewire + colon), 6e73e418 (grammar), 6e547e3b (lockstep), 175a5cf6+086acdb2 (audit)
