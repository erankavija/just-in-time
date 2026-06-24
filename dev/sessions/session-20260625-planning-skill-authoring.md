# Session Notes: jit-plan Skill Authoring (2026-06-25)

**Issue:** `eed6750c` — Planning skill: interview-driven plan authoring for the P node

**Type:** Implementation (skill authored). Dogfooding deferred to a separate unprimed
session by design.

**Status:** `jit-plan` skill drafted and self-reviewed against REQ-01..06. `eed6750c`
left claimed/in-progress so dogfood-surfaced gaps fold in before the gate.

---

## What was built

```
.claude/skills/jit-plan/
  SKILL.md                       # orchestrator, 260 lines / ~3.7k tok (REQ-04 budget: <500 / ~5k)
  references/
    interview-protocol.md        # ingest-then-interview, strict DoD floor, decision log
    investigator-prompt.md       # ground claims in real code; done/valid-open/invalid; consumer sweep; verify primitives
    researcher-prompt.md         # conditional (signal-gated); cited research doc linked to P, separate from plan
    synthesizer-prompt.md        # findings+intent → plan to the 4 plan-review areas + decision log + near-ready sketch
    reviewer-prompt.md           # adversarial, code-grounded pre-gate self-review against the 4 areas
    plan-doc-template.md         # the 4 sections + Decisions log + [hard] REQ-NN format + sketch
```

Symlinked at `~/.claude/skills/jit-plan` to match the sibling convention; the harness
lists it as active. Grounded in the live config (read, never hardcoded): the `plan`
template in `.jit/templates.toml` (breakable `applies_to`, P/B node types, P's plan-doc
location, gates) and the coverage rule in `.jit/rules.toml`
(`success_criteria` / `[hard]` / `REQ-[0-9]+` / `satisfies`).

## Method: dogfooded the interview on the skill itself

Per the user's directive ("interview me thoroughly on any details and decisions"), the
skill's open design forks were settled by running the planning interview on the user
before authoring — exercising the skill's own decision-log / intent-first philosophy. The
elicited decision log (each with the rejected options, as the protocol mandates):

| # | Decision | Rejected |
|---|----------|----------|
| 1 | Name `jit-plan` | jit-spec (overlaps breakdown), jit-design (collides with the `design` classification) |
| 2 | Self-contained, domain-agnostic ingest **+ optional ingester hook** | hard-delegate to research-librarian (not always present, couples skills); pure self-contained (no extensibility) |
| 3 | jit-plan **creates the container directly**; authoring standards cite the single canonical `jit-manage/references/content-standards.md` (SSOT) | delegate creation to jit-manage (extra hop) |
| 4 | **Near-ready child specs** in the sketch (titles, one-outcome, tiers, `[hard]` markers, `satisfies:REQ-NN`, ordering) | clusters-only; middle |
| 5 | **Owner runs its own** plan-review gate to a recorded verdict (supersedes design-doc §7) | leave PENDING for a "standard runner" (the corrected misconception) |
| 6 | **Strict DoD floor + escalate**; ≥1 owner-confirmed `[hard]` REQ; record assumptions with rationale; never fabricate | moderate (flagged defaults); light (infer + confirm once) |
| 7 | **Stop at the sketch** on PASS; mark P done; no child creation | offer-to-chain; auto-chain into breakdown |
| 8 | **Interactive-first**, headless path thin (composition into a lead is a non-goal/follow-up) | build both fully; interactive-only |
| 9 | **Separate conditional research artifact**, signal-gated, cited, linked to P | fold into investigator; optional output |
| 10 | **Adversarial, code-grounded** pre-gate reviewer; fix ALL findings | light rubric checklist |

**Logged follow-up (out of scope, REQ-06 forbids touching jit-manage):** promote
`content-standards.md` from `jit-manage/references/` to a neutral shared location so all
skills cite one SSOT without a cross-skill reach.

## Lessons encoded (from the three prior session docs)

The orchestrator + references fold in the validated observations (ground in current state;
living decision log; provisional decisions; atomic addressable criteria; size-matched
intermediate groupings; bounded blast radius + ripple handling; standalone-readable,
relationships-in-graph; adversarial gate loop) and the failure disciplines (verify
criteria vs the live system + sweep prior docs; verify named primitives; exhaustive
consumer sweep; intent-before-mechanics; capture-after-convergence; purpose-only
descriptions; owner-runs-its-gate; explicit markers; conventions from versioned docs;
landable waves + grep acceptance; bounded gate loop reading the recorded verdict;
explicit plan→breakdown handoff with repo-wide validation coupling; orchestrator size
budget).

## REQ self-review

- **REQ-01** cold-start/ingest/interview/converge, no fabrication, escalate → Phase 2 +
  interview-protocol §1/§5 + Invariant 3 + headless variant.
- **REQ-02** create breakable container (type from `applies_to`, never hardcoded),
  `[hard] REQ-NN` in `## Success Criteria`, bracket P at configured plan-doc → Phase 3 +
  Invariant 1. (REQ-02's `[planning].breakable_types` now resolves to the `plan`
  template's `applies_to`, per design-doc §7's HISTORICAL note.)
- **REQ-03** plan to 4 areas, self-review, run plan-review to recorded passed before
  breakdown → Phases 5–8.
- **REQ-04** orchestrator <500 lines/~5k tok; investigate/synthesize/review dispatched in
  `references/` → 260 lines / ~3.7k tok; prompts present.
- **REQ-05** recurse one level, sketch only, children remain jit-breakdown → Invariant 9 +
  Phase 8.3.
- **REQ-06** no existing skill modified → `git status` shows only `jit-plan/` added.

## Side-correction: `jit gate pass` exit code

Verified against `main.rs:33-42` `error_to_exit_code`: the exit code is now
verdict-meaningful — `0` pass, `4` FAIL (`ValidationFailed`), `10` runner error
(`ExternalError`). The stale memory (`exits 0 even on FAIL`) was corrected; the durable
caution narrows to "prefer the recorded gate status over an outer wrapper's success
signal." The skill's Phase 7 reflects this.

## Next

- Dogfood `jit-plan` in a **separate unprimed session** to test triggering and guidance
  without context priming; then analyze the session record and fold any gaps back into
  `eed6750c` (dogfood-findings-fix-in-epic) before running the `code-review` gate and
  completing the task.

## References

- Issue `eed6750c`; dependency epic `2fbd2a82` (bracket); graph-templates epic `9ac9fdac`.
- Prior session docs: [design](session-20260620-planning-skill-design.md),
  [observations](session-20260623-planning-skill-observations.md),
  [failures/churn](session-20260622-planning-failures-and-churn.md).
- Live config: `.jit/templates.toml` (`plan`), `.jit/rules.toml` (`bracket:coverage-preview`),
  `crates/jit/src/gate_presets/planning.rs`, `scripts/plan-review-prompt.md`.
