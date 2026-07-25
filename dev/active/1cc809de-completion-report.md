# Epic Complete: Repository-state quality hardening (1cc809de)

**Started:** 2026-07-24
**Completed:** 2026-07-25
**Assignee:** agent:jit-execution-lead

## Summary

Hardened the transactional repository-materialization subsystem delivered by epic `cdc840ad` from a state where its strongest guarantees were narrated to one where they are enforced, acting on the adversarial audit `dev/studies/cdc840ad-audit-2026-07-23.md`. The planning boundary now matches its claim behind a single mutation-session combinator; the guarantees are asserted by property-based and multi-threaded contention tests; performance is an artifact-backed contract rather than a self-attested figure; the recorded hygiene debt is cleared and the subsystem documented; and the predecessor showcase deck is replaced by a corrected successor whose every falsifiable claim traces to a repository artifact.

Completion evidence is a gate, not a narrative: `repo-validate` and `holistic-review` both passed at `35ba4eb0`, the latter performed by a reviewer distinct from every building agent, per `@/charter/D-5`.

## Metrics

| Metric | Value |
|---|---|
| Subtree nodes completed | 38 / 38 (36 issues + 2 bracket nodes), all Done |
| Stories completed | 5 / 5 |
| Waves executed | 9 (8 planned + 1 post-review remediation) |
| Rework cycles | 18 across 11 issues |
| Escalations | 6 (all resolved; 5 by invoker guidance, 1 handled autonomously) |
| Sub-agent dispatches | 34 distinct workers, including reworks |
| Issues created during execution | 1 (`293ce118`) |
| Pitfalls recorded for follow-up | 14 |

Final suite at completion: `cargo-ci` 3867 passed, 0 failed, 14 ignored.

## Success Criteria

All 16 hard criteria map one-to-one onto the five story checkpoints; each story completed only after its whole subtree did. The independent `holistic-review` verdict states: *"All 16 hard criteria are verifiably met; two non-blocking advisories remain."*

- [x] REQ-01…REQ-04 — single mutation-session combinator owning retry/conflict classification, typed producer errors with no stringify-then-downcast, total journal-action extraction with no `unreachable!` on the publication path, plan-identity tail for initialize and profile-application — delivered by story `d159f9d4` (`4b2005fe`, `eefbfe84`, `9fda1f86`, `fee3c528`, `a4e5ca3d`, `a6ee4e23`, `9298b912`, `58c2f0e9`)
- [x] REQ-05…REQ-08 — plan-hash reorder-invariance and managed-document splice properties, visibility-enforced cutover guard, multi-threaded contention tests, compile-time path constants with derived repair coverage — delivered by story `2958105e` (`5b121c3f`, `ab69a4ef`, `7ab27a9b`, `8becd4a1`, `dd5b43b6`, `28254964`, `1781aec2`, `67a77503`) and completed by `293ce118`
- [x] REQ-09…REQ-11 — checked-in benchmark harness recording repeated runs with machine and cache-state metadata, session-open budget for bulk update, lock hygiene chosen from a recorded cost profile — delivered by story `d752293b` (`73981310`, `fc744df6`, `412925b9`, `a4b0fadf`)
- [x] REQ-12…REQ-15 — feature-gated test-support surface with dead exports removed, store test module split and review-round test names corrected, contributor architecture document with the stale storage pointer swept, five advisory-debt items cleared — delivered by story `755cf453` (`27c8256a`, `39e1c091`, `531ce80d`, `9503c8b4`, `e6d5e440`, `eb4be05c`, `73d9070f`, `42ee0dd5`)
- [x] REQ-16 — corrected successor deck and predecessor tombstone — delivered by story `9ee14023` (`f87e3273`, `cba48167`)

## Wave Execution Log

**Wave 1 (6)** — S1 and S3 foundations: retry combinator, total journal-action extraction, typed producer errors, plan-identity tail, benchmark harness with the first session-cost artifact, per-issue sidecar read-lock removal.
**Wave 2 (2)** — S1 mid-layer: single-session call-site migration across ~15 command files, typed finalizer errors.
**Wave 3 (3)** — S1 sinks plus the bulk-update eligibility prefilter and session budget.
**Wave 4 (3)** — bulk-mutation benchmark scenario, then story checkpoints S1 and S3.
**Wave 5 (12)** — S2 and S4 first layer: two property tests, `Cow`-backed `VirtualPath` with well-known constants, derived repair-target authority, contention tests, test-support feature gate, MCP warning cleanup, the architecture document, fail-closed CI audits, dead-parameter removal, store test split, CLI error-code correction.
**Wave 6 (6)** — S2 and S4 sinks: 99-site path-constant migration, visibility-enforced cutover guard, dead-export deletion with surface demotion, rule-serialization gating; then story checkpoints S2 and S4.
**Wave 7 (1)** — corrected successor deck: 31 slides, vendored reveal.js 5.1.0 and eight woff2 faces, no math library, one slide-number chrome, fit-checked under both themes.
**Wave 8 (2)** — predecessor deck archived behind a tombstone; then story checkpoint S5.
**Wave 9 (1)** — post-review remediation of REQ-08 after the epic's first `holistic-review` failed. See below.

## Key Decisions

- **Direct-main delivery with per-issue worktree isolation.** Every wave landed reviewed final-form changes on `main`; worker branches were temporary isolation only. Each merge was preceded by a leak check, and every code-bearing merge commit was build-verified from sources resolved in isolation.
- **Gate evidence is commit-exact.** Gates ran one at a time from the repository root through a background chain that reinstalled the dogfood binary after every evidence commit, so the stale-binary guard stayed effective. Evidence was committed whether the gate passed or failed, and both epic gates were re-run at the final HEAD rather than relying on an earlier pass that predated wave 9's code.
- **Class-wide rework over single-finding patches.** After `531ce80d` spent four rounds while the reviewer walked one audit-threshold vector per round, every later rework carried an exhaustiveness sweep. The whole-surface rework passed immediately, and the same discipline closed `f87e3273` in two rounds.
- **Story reviews read the story's criterion, not the union of task criteria.** This caught residual conflict arms in `d159f9d4`, two review-round-named tests in `755cf453`, and a non-discriminating property test in `2958105e` — each of which every task-level review had passed.
- **Workers were forbidden from writing `.jit`** beyond `jit doc add` on their own issue. This kept state changes auditable, but meant the lead had to close consequences workers structurally could not reach — see the archival finding below.
- **A failed gate outranks the plan.** When `holistic-review` failed on REQ-08, two review gates appeared to conflict: `breakdown-review` had held the plan authoritative and the plan's enumerated const set excluded `config/gate-presets`. Resolved without escalation on the grounds that the plan binds the fan-out shape but cannot make an unmet criterion met.

## Escalations

Six, all resolved; none left open.

1. **`9fda1f86` — external-review authorization.** The `code-review` gate sends repository code to an external AI reviewer. Authorized explicitly by the invoker.
2. **`412925b9` — rework cap.** Three rounds each found a distinct real defect (non-session preview skip authority, a missing Done-redirect gate check, a verification error aborting a best-effort loop). Invoker authorized a targeted fourth attempt and amended REQ-01/02/04 with the `C*J+1` session budget, recorded as `@/issue/412925b9/decision/D-1`.
3. **`531ce80d` — rework cap, twice.** The reviewer walked the npm and cargo audit threshold ladder across five rounds. Invoker authorized an explicit `--audit-level=info` fix, then a whole-surface fail-closed sweep.
4. **`73d9070f` — issue scope.** REQ-02 named two items for demotion on the premise they had no consumer outside `repository_state`; both had one. Invoker chose the test-support twin pattern, satisfying the criterion on the default surface with no criteria amendment.
5. **`1cc809de` — gate failure expanding scope, handled autonomously.** The epic's `holistic-review` failed with one blocking finding. Creating a remediation task inside the epic is an autonomous action, so the lead created `293ce118` rather than escalating, and reported the plan-versus-criterion tension.

## Issues Discovered During Execution

**`293ce118` — Make the gate-presets path a constant and the known-path inventory self-enforcing** (created during wave 9). The epic's independent `holistic-review` failed with a blocking finding that REQ-08 was unmet: `config/gate-presets` still reached production as a runtime string, and `VirtualPath::ALL_KNOWN` was a hand-maintained mirror whose own doc comment admitted the coverage gap. Both halves had been recorded as deferred follow-ups during wave 6 on plan-authority reasoning.

The fix makes `config/gate-presets` and its parent directory `VirtualPath` consts, and emits the fourteen well-known consts together with `ALL_KNOWN` from one `declare_well_known_paths!` declaration. The struct then moved into the declaration module, which hands its parent only a `Result`-returning constructor — unusable in a const initializer — plus two accessors.

Fourteen further findings are recorded as `surfaced_pitfalls` in the progress file rather than fixed mid-flight, because the approved plan binds the fan-out. The highest-value candidates:

- **`VirtualPath` has no join/child API**, so ~78 composite production sites keep the fallible constructor, and `InMemoryStorage::issue_vpath` (`storage/memory.rs:340`) degrades it to a latent panic on an id containing `/`, `..`, or a control character. This violates the no-panics-in-library-code convention and is advisory finding F1 of the passing holistic review.
- **`dev/presentations` is outside `[documentation].managed_paths`**, so deck archival cannot use the product's own archival workflow. Advisory finding F2. See the archival note below.
- **The test-support twin pattern has no rustdoc guard.** Gating an item flips it to `pub(crate)` on the default surface, silently breaking intra-doc links; no configured gate runs `cargo doc`, and ~93 pre-existing warnings already bury the signal.
- **Two independent temp-name counters** (`storage/atomic_write.rs:23`, `storage/external_publish.rs:31`) mint names of the same shape; one path uses raw `fs::write` and would silently overwrite. Adjacent to `@/inv/atomic-writes`.
- **`default_rule_membership_diff` has no production caller** — measured, not inferred — and is kept alive only by its own tests and doctest.
- **The provenance-contract suite's `production_source` helper** still splits source text on a literal `#[cfg(test)]` spelling, so a rustfmt change silently widens or narrows what three retained tests see.

## Holistic Quality Notes

- **The epic's own thesis held against its execution.** Narrated-versus-enforced was the deliverable, and the review process reproduced the audit's central finding three times on this epic's own work: a property test seeded so it could not fail (`2958105e`), a deck slide asserting an evidence policy it did not follow (`f87e3273`), and a const inventory claiming enforcement it did not have (`293ce118`). Each was caught by reading the artifact rather than the claim, and the last one was caught by the independent gate after the lead had already accepted the work.
- **A lead-review blind spot, twice.** Tier 2 verifies that an artifact's claims are *true*; it does not verify that an artifact declaring its own policy *honors* it. The lead verified all five completion-evidence facts against artifacts and still passed a slide citing none of them. Separately, the lead accepted an enforcement claim after one compiler probe and only found the surviving hole by running a second probe the worker had not. Both are cheap to prevent: sweep every section of a self-policing artifact for a provenance cite, and probe every route to the thing a guarantee claims to forbid.
- **An inventory that becomes a spec must never be truncated.** The lead built `293ce118`'s migration-site list with `git grep … | head -10`, and the cap silently dropped two production sites. Only the prompt's instruction to treat the list as a starting point rather than an authority kept the migration complete.
- **The archival mechanism was bypassed, and the plan is why.** jit ships first-class artifact archival (`jit archive document|container --execute`) whose plan carries `reference_changes` and whose executor calls `remap_archived_references` (`commands/archive.rs:68`), so issue document references follow a moved artifact. `cba48167` instead hand-rolled the move with `git mv` to a destination REQ-01 pinned as `dev/archive/features/cdc840ad/showcase/`. The automatic remapping therefore never ran: epic `cdc840ad`'s deck reference dangled and `jit validate` hard-failed until the lead repointed it. The mechanism would also have refused this move, since `artifact_classifier.rs:838-850` blocks a selected root that is neither managed, permanent, nor already archived. Decks are exactly the artifact class this epic archives, yet they fall outside the paths the archival policy manages. Recommended follow-ups: add `dev/presentations` to `managed_paths`, or teach the archival planner the feature-archive shape.
- **Link checkers only see links out of a document.** A move also needs a check for references *into* the moved paths (`git grep` plus `jit validate`). The tombstone worker verified its own outbound links three ways and still could not have caught the inbound break.
- **`28254964` carries a label/parent mismatch.** It satisfies REQ-12, an S4 criterion, but is DAG-parented under story `2958105e` in S2. Structurally harmless — the ordering is correct because it is a shared foundation consumed by S2's cutover guard and S4's demotions — but the membership label disagrees with the hierarchy parent. `jit query divergence` reports the tree clean, because this pairing predates that check's scope.
- **`cargo audit` has no CLI severity-threshold override**, so a machine-local `~/.cargo/audit.toml` could suppress low-CVSS advisories. Flagged by a worker; unfixable from the script.
