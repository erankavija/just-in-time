# Breakdown spec for story c23dfe71

## Story description (authoritative for this story)

Makes "the renamed skills pass their evals" a checkable claim rather than an aspiration: defines how a scenario eval is adjudicated, runs it once per renamed skill to record a passing baseline, and adds an activation (trigger) check for each.

## Background

`jit-execution-lead` ships three scenario evals (`evals/evals.json`) that grade free-text `expected_output` against actual agent behavior, but no runner exists in-repo to execute and adjudicate them. `jit-planning-lead` has no `evals/` directory at all. Neither skill's activation-from-description has ever been checked with a trigger eval.

## Success Criteria

- [hard] REQ-01: A documented adjudication method exists for scenario evals — either an LLM-judge procedure that grades a recorded transcript against the `expected_output` rubric and records a verdict, or each eval's `expected_output` converted into an assertable checklist. The method is written down, not performed once from memory.
- [hard] REQ-02: `jit-execution-lead`'s three existing scenario evals (`evals/evals.json`) are run under the adjudication method and the passing verdicts are recorded.
- [hard] REQ-03: `jit-planning-lead` gains an equivalent `evals/` directory with scenario evals covering its documented entry paths, run under the same adjudication method with recorded passing verdicts.
- [hard] REQ-04: Both renamed skills gain an activation (trigger) check — a `trigger_eval.json` consumed by the skill-creator plugin's trigger-eval runner, or a documented equivalent — with recorded passing results confirming each skill activates from its own description.

## Notes

Scope this to a thin, documented run-and-record procedure plus the minimal machinery each check needs — not a reusable eval framework. A general framework is explicit scope creep past what "pass their evals" requires.

## Parent-plan decomposition sketch (relevant group)

### Group A: Rename completion and eval verifiability — covers REQ-07
- **Sweep residual pre-rename references**  `type: task`  `satisfies: REQ-07`  `depends-on: —`
  Outcome: no live pre-rename references remain; the 9 verified hits are corrected
  (jit-execution-lead `SKILL.md:14`, jit-planning-lead `SKILL.md:9,11`, both dispatch scripts
  line 2, `dev/active/eed6750c-handoff.md:1,7,44,48` in full).
  Own criteria: `[hard] REQ-A1: the acceptance check (below) returns zero hits over the live
  scope.`
  Blast radius: jit-execution-lead `SKILL.md`, jit-planning-lead `SKILL.md`, both dispatch
  scripts, `dev/active/eed6750c-handoff.md` (all 4 hits); historical records exempt per the
  acceptance-check rule, untouched.
- **Runnable eval verification for the lead skills**  `type: story`  `satisfies: REQ-07`  `depends-on: Sweep residual pre-rename references`
  Outcome: scenario evals are runnable with defined pass adjudication, both renamed skills
  have recorded passing baselines, and activation is verified by trigger evals.
  Own criteria:
  `[hard] REQ-A2: each renamed lead has scenario evals with a defined adjudication method
  (LLM-judge against the expected_output rubric with recorded transcript and verdict, or
  expected_output converted to assertable checklists) and a recorded passing baseline
  reproducible by a documented procedure.`
  `[hard] REQ-A3: both renamed skills have activation verification (a trigger_eval.json
  consumed by the skill-creator plugin's trigger-eval runner, or a documented equivalent
  trigger check) with recorded passing results.`
  Blast radius: `jit-execution-lead/evals/`, new `jit-planning-lead/evals/`, trigger-eval
  definitions; no engine change.


## Parent-plan technical grounding (§2, for citations)

## 2. Technical soundness and architectural fit

- **Approach:** A skills-only addition (markdown + shell), one strategic tier above
  jit-execution-lead. It reuses existing CLI-exposed primitives and existing skill
  protocols; it reaches into no `crates/jit` domain/storage/graph code. It is compositional:
  jit-project-lead dispatches jit-execution-lead as a subagent, never re-implementing epic
  execution (Non-goals).

- **Reuses / integrates with:**
  - Tier derivation: `strategic_types` at `.jit/config.toml:31` (consumed by `jit query
    strategic`); breakable-container types from `applies_to` in `.jit/templates.toml:12`;
    `jit config show-hierarchy --json` for the type→level map; `[type_hierarchy]` at
    `.jit/config.toml:24-31`. Divergent ruleset exercised: `docs/examples/research/config.toml:34`
    (`strategic_types = ["goal"]`) with `docs/examples/research/templates.toml:18`
    (`applies_to = ["goal"]`).
  - Dispatch: `.claude/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh` and
    `.../scripts/check-leak-into-main.sh`, documented canonical-copy-only in
    `references/worktree-dispatch-protocol.md` (subagent invokes them; no fork).
  - Waves: `jit graph deps <id>` hand-layered per jit-execution-lead `SKILL.md:146-172`.
  - Coherence review: `references/lead-review-protocol.md:92` (Tier 3), `:57`
    (stale-forward-reference sweep), a one-tier-up analogue.
  - Escalation: single locus `references/escalation-policy.md:14` plus SKILL wording sites.
  - Artifacts: `jit doc add/list/show/history` for linking; progress shape from
    `dev/active/7095769d-progress.json`; documentation config at `.jit/config.toml:9-14`.
  - Cold-start: jit-planning-lead Step 2 interview + Step 5 recursion; milestone containers
    fall back to jit-breakdown's plain-breakdown path (`jit-breakdown/SKILL.md:105-109`)
    since `.jit/templates.toml:12` binds the only template to `["epic"]`.
  - Standards: canonical-ish `jit-manage/references/content-standards.md` (168 lines),
    referenced by `jit-execution-lead/references/doc-agent-prompt.md:42`,
    `architect-agent-prompt.md:37`, `jit-breakdown/references/analysis-prompt.md:106`;
    byte-identical duplicate at `jit-planning-lead/references/content-standards.md`.
    Canonical home after promotion: `docs/reference/jit-content-standards.md`.
    Cross-project reachability: every jit entry in `~/.claude/skills` is a per-skill
    symlink into this repo's `.claude/skills` (verified `ls -la ~/.claude/skills`), so a
    skill file's physical location after symlink resolution is
    `<this-repo>/.claude/skills/<skill>/...`, and a skill-base-relative traversal
    (`../../../docs/reference/jit-content-standards.md` from the skill dir, one level
    deeper from `references/` files) resolves inside this repo even when the skill is
    entered from another project (verified:
    `realpath ~/.claude/skills/jit-execution-lead/../../../docs/reference` →
    `/home/vkaskivuo/Projects/just-in-time/docs/reference`).

- **Grounding (from investigation), classified:**
  - Rename → **partially already-done**: dirs/symlinks/most refs done (751da7f5); 9 residual
    live hits remained (jit-execution-lead `SKILL.md:14` H1; jit-planning-lead `SKILL.md:9`
    H1 and `:11` opening line; both scripts' line-2 header comment;
    `dev/active/eed6750c-handoff.md:1,7,44,48`, of which line 48's pre-rename skill-directory
    path was the operationally load-bearing pointer); swept in c6325c5b.
  - Eval-pass → **invalid-as-stated (unverifiable today)**: jit-execution-lead
    `evals/evals.json` has 3 scenarios but no runner in-repo; jit-planning-lead has no
    `evals/`. Plan makes it verifiable.
  - Config-derived tiers (REQ-01), four modes (REQ-02), durable artifacts (REQ-03),
    dispatch (REQ-04), escalation (REQ-05), canonical standards (REQ-06) →
    **valid-and-open**, primitives cited above exist.
  - Domain-agnosticism: engine hardcodes no domain type (`.jit/templates.toml` header
    comment; project-declared `type_hierarchy`). The divergent ruleset the derivation must
    handle is the in-repo research example: `docs/examples/research/config.toml:34` declares
    a single strategic type (`goal`) that is itself the breakable container
    (`docs/examples/research/templates.toml:18`), so no second strategic tier exists to
    derive. `../gf2` uses the same `milestone`/`epic` pair as this repo and adds no third
    shape.
  - Layer boundary: everything needed is CLI-exposed; skills-only change, consistent with
    CLAUDE.md layering.


## Parent-plan decisions

## Decisions

First-class log, consumed by review and breakdown. Provisional entries flagged.

- **D1 — Rename target for the epic lead:** chosen **`jit-execution-lead`** (RESOLVED
  2026-07-02 in the container). Rejected: `jit-task-lead` (collides with the `task` tier),
  `jit-epic-lead` (hardcodes this repo's sub-strategic type), `jit-delivery-lead` /
  `jit-initiative-lead` / `jit-outcome-lead` (viable but less precise about the execution
  function). The planning skill was correspondingly renamed `jit-planning-lead`.
- **D2 — `-lead` suffix reserved for autonomous standing roles:** chosen. Workflow skills
  (`jit-manage`, `jit-breakdown`, `jit-parallel`, `jit-migrate`) stay un-suffixed.
- **D3 — Canonical standards home (final):** chosen
  **`docs/reference/jit-content-standards.md`** (versioned, permanent per
  `permanent_paths = ["docs/"]`, in the existing `docs/reference/` Diataxis area), consumed
  by both leads and the sweep mode via direct skill-base-relative path references
  (`../../../docs/reference/jit-content-standards.md` from a skill dir, one level deeper from
  `references/` files); both existing copies are removed in the same change, direct
  references rather than pointer stubs (stubs recreate the multi-file indirection REQ-06
  removes and can drift). Reachability holds cross-project because every `~/.claude/skills`
  jit entry is a per-skill symlink into this repo's `.claude/skills`; verified resolution
  argument in §2. Rejected: keep the canonical copy inside jit-manage's `references/`
  (couples the project-wide SSOT to one skill's directory and leaves the duplicate problem);
  synced copies (violates "single canonical doc"); leave the byte-identical duplicate (the
  scattered state REQ-06 targets).
- **D4 — Vision/charter storage:** chosen **`permanent_paths = ["docs/"]`** (never
  auto-archived). Rejected: a `managed_paths` location under `dev/active` (would be archived by
  `jit doc archive` on issue completion, wrong for a never-terminal milestone vision); adding a
  new `[documentation.categories]` entry (larger config change, out of proportion to need).
- **D5 — Eval verifiability scope:** chosen **a thin documented run-and-record procedure plus
  minimal machinery** to make "pass their evals" checkable. Rejected: build a general eval
  framework (scope creep beyond REQ-07); leave evals unrunnable (REQ-07 stays unverifiable).
- **D6 — Re-home the eed6750c dependency (provisional; reviewer must see):** chosen **move the
  edge to the mode-2/cold-start item and remove P→eed6750c** at breakdown. Rationale: the
  dependency's semantic content is "modes 2/3 consume jit-planning-lead". P's deliverable is
  this plan document, which does not require jit-planning-lead to exist; mode 2 does, so the
  edge belongs on the mode-2 item. eed6750c is InProgress with its code-review gate never run
  and is not closeable within this epic. Rejected: drive eed6750c to Done inside this epic
  (out of scope per Non-goals); leave P blocked on eed6750c (blocks a deliverable that does
  not consume the blocker).
- **D7 — Compositional design:** chosen. jit-project-lead dispatches jit-execution-lead as a
  subagent per sub-strategic container in topological waves; it never re-implements epic
  execution. Rejected: re-implementing epic breakdown/execution at the new tier (violates
  Non-goals and duplicates the canonical dispatch scripts).
- **D8 — Skill prose style:** chosen the **cc-sdd / superpowers register** for new and
  modified skill files: terse imperative voice, structured sections with explicit success
  criteria and safety/fallback paths, explicit stop-and-escalate conditions, bounded review
  loops, red-flag lists, rules split into `references/` read on demand, no filler. Exemplars
  every child issue's author consults: `../jit-research/cc-sdd/tools/cc-sdd/templates/agents/claude-code-skills/skills/kiro-spec-design/SKILL.md`;
  `../jit-research/superpowers/skills/writing-plans/SKILL.md`;
  `../jit-research/superpowers/skills/executing-plans/SKILL.md`.
- **D9 — Strategic-tier bracketing deferred:** whether to add `milestone` to a template's
  `applies_to` (`.jit/templates.toml:12` binds only `["epic"]`) is an OPEN question explicitly
  out of scope for this epic. Mode 2 uses jit-breakdown's plain-breakdown fallback
  (`jit-breakdown/SKILL.md:105-109`) for milestone containers. No work planned for bracketing.

- **Assumptions:** (a) `strategic_types` lists entries most-strategic first (holds in both
  observed rulesets) and the breakable types in `applies_to` sit at or below the anchor's
  level; risk: a ruleset violating either yields an ambiguous anchor/boundary split, and the
  skill must then stop and ask rather than guess (the fallback path in the §1 REQ-01 rule).
  (b) The milestone progress file mirrors `dev/active/7095769d-progress.json` one tier up
  (per-epic rows); risk: milestone-specific fields surface during mode 1; amendable in-loop.
