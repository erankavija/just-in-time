# Breakdown spec for story e8b1cee3

## Story description (authoritative for this story)

Creates the `jit-project-lead` skill: a thin orchestrator `SKILL.md` plus role prompts in `references/`, and the logic that derives its strategic and sub-strategic tiers, and the container types it dispatches an execution lead for, from the project's own configuration rather than any hardcoded domain vocabulary.

## Background

jit backs software projects (this repo, where the hierarchy is `milestone` > `epic`) and non-software ones (a research example where `strategic_types = ["goal"]` and `goal` is itself the breakable container — there is no second strategic tier). The skill must derive its "steward anchor" (the most-strategic type, read from `strategic_types` in `.jit/config.toml`) and its "delegation boundary" (the container types it dispatches an execution lead for — the union of `applies_to` across `[[template]]` entries in `.jit/templates.toml`) from each project's own configuration, falling back to the numeric type-to-level map (`jit config show-hierarchy --json`) when `templates.toml` is missing or `strategic_types` is empty. Prose refers to "strategic tier" / "sub-strategic tier" as placeholders, never literal `milestone`/`epic`/`goal` names — the same convention `jit-execution-lead` already uses for `<planning_type>`/`<breakdown_type>`.

## Success Criteria

- [hard] REQ-01: `.agents/skills/jit-project-lead/SKILL.md` exists as a thin orchestrator under 500 lines (roughly 5k tokens), with role prompts and detailed rules split into `references/` files read on demand.
- [hard] REQ-02: The skill derives its steward anchor and delegation boundary from `[type_hierarchy]`/`strategic_types` in `.jit/config.toml` and `applies_to` in `.jit/templates.toml`, with zero hardcoded domain type literals (no `milestone`, `epic`, or `goal` string anywhere in the skill's control-flow logic).
- [hard] REQ-03: Given this repository's ruleset (two strategic tiers: `milestone` anchor, `epic` delegation boundary) the derivation yields the correct anchor and delegation boundary; given the research example ruleset (`docs/examples/research/config.toml`, `strategic_types = ["goal"]`, `goal` also the breakable type) it yields the collapsed case correctly — one strategic tier, the steward's scope is the portfolio of top-level `goal` containers.
- [hard] REQ-04: When `templates.toml` is missing, `strategic_types` is empty, or the derivation would leave a genuine level tie among candidate anchors, the skill stops and asks rather than guessing.
- [hard] REQ-05: The skill activates from its own `description` field (verified by a trigger-eval check) and its scenario evals pass, both under the adjudication method established for the other lead skills.

## Notes

Skill prose (SKILL.md and references/) follows the cc-sdd/superpowers register: terse imperative voice, explicit success criteria and safety/fallback paths, explicit stop-and-escalate conditions, bounded review loops, red-flag lists over prose warnings, detail rules pushed to `references/` read on demand, no filler. Exemplars: `../jit-research/cc-sdd/tools/cc-sdd/templates/agents/claude-code-skills/skills/kiro-spec-design/SKILL.md`, `../jit-research/superpowers/skills/writing-plans/SKILL.md`, `../jit-research/superpowers/skills/executing-plans/SKILL.md`.

The 500-line budget is tight — `jit-execution-lead` already spends 358 of its 500 lines on narrower scope. If the orchestrator approaches budget during authoring, extract mode bodies (steering discussion, standards sweep) into `references/`, leaving routing stubs in `SKILL.md`.


The skeleton delivers the orchestrator shell (pre-flight, tier derivation, references/ scaffolding) and leaves the mode-routing block and mode bodies to follow-up work — it does not author them.

## Parent-plan decomposition sketch (relevant group)

### Group C: jit-project-lead skeleton, tiers, modes, artifacts — covers REQ-01, REQ-02, REQ-03
- **jit-project-lead skeleton with config-derived tiers**  `type: story`  `satisfies: REQ-01`  `depends-on: Runnable eval verification for the lead skills, Promote content standards to one canonical doc`
  Outcome: the skill exists with a thin `SKILL.md` (under 500 lines) plus `references/`,
  derives steward anchor and delegation boundary from the project's config and templates
  (the §1 REQ-01 rule), hardcodes no domain type name, activates from its description, and
  passes its evals.
  Own criteria: `[hard] REQ-C1: skill activates from its description (verified with the
  Group A trigger-eval mechanism); SKILL.md under the budget; tiers resolved from config with
  zero hardcoded type literals; evals pass under the Group A adjudication method; the
  derivation yields correct anchor and delegation boundary on both observed rulesets, the
  two-tier .jit/config.toml and the collapsed single-tier docs/examples/research ruleset
  (strategic type equals the breakable type).`
  Blast radius: new `.agents/skills/jit-project-lead/`, `~/.agents/skills` symlink.
- **Four-mode front door with request routing**  `type: story`  `satisfies: REQ-02`  `depends-on: jit-project-lead skeleton with config-derived tiers`
  Outcome: an opening request routes to one of the four modes; mode 1 is fully functional;
  modes 2/3 delegate to jit-planning-lead; mode 4 delegates to the sweep path.
  Own criteria: `[hard] REQ-C2: representative requests for all four modes route correctly;
  mode 1 executes end to end.`
  Blast radius: self-contained to the new skill. At breakdown, add the `depends-on` edge from
  this item (its mode-2/cold-start surface) onto eed6750c and remove P→eed6750c (see Decisions).
- **Durable vision/charter and resumable progress artifacts**  `type: story`  `satisfies: REQ-03`  `depends-on: jit-project-lead skeleton with config-derived tiers`
  Outcome: a vision/charter doc (vision + decision log of chosen/rejected options with
  reasons) at a documentation-config-derived permanent location, plus a milestone-level
  progress file; both link to the milestone and survive across sessions.
  Own criteria: `[hard] REQ-C3: re-invocation reads back the vision/charter and progress state
  and resumes without loss.`
  Blast radius: new doc artifacts + `jit doc` links; self-contained.


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
  - Dispatch: `.agents/skills/jit-execution-lead/scripts/dispatch-worker-worktree.sh` and
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
    Cross-project reachability: every jit entry in `~/.agents/skills` is a per-skill
    symlink into this repo's `.agents/skills` (verified `ls -la ~/.agents/skills`), so a
    skill file's physical location after symlink resolution is
    `<this-repo>/.agents/skills/<skill>/...`, and a skill-base-relative traversal
    (`../../../docs/reference/jit-content-standards.md` from the skill dir, one level
    deeper from `references/` files) resolves inside this repo even when the skill is
    entered from another project (verified:
    `realpath ~/.agents/skills/jit-execution-lead/../../../docs/reference` →
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
    AGENTS.md layering.


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
  removes and can drift). Reachability holds cross-project because every `~/.agents/skills`
  jit entry is a per-skill symlink into this repo's `.agents/skills`; verified resolution
  argument in §2. Rejected: keep the canonical copy inside jit-manage's `references/`
  (couples the project-wide SSOT to one skill's directory and leaves the duplicate problem);
  synced copies (violates "single canonical doc"); leave the byte-identical duplicate (the
  scattered state REQ-06 targets).
- **D4 — Vision/charter storage:** chosen **`permanent_paths = ["docs/"]`** (never
  auto-archived). Rejected: a `managed_paths` location under `dev/active` (would be archived by
  the retired category-based archiver on issue completion, wrong for a never-terminal milestone
  vision); adding another category entry (larger config change, out of proportion to need).
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
