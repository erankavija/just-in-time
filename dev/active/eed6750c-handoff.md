# Handoff: jit-plan skill (eed6750c)

**State:** skill authored, dogfooded once (reconcile path), post-dogfood fixes folded in.
`eed6750c` is **in-progress / claimed** (`agent:claude`) — **not gated, not done**.

## Done
- `jit-plan` skill: `SKILL.md` orchestrator (265 lines / ~3.9k tok, under the REQ-04 budget)
  + six `references/`. Initial skill in commit `23dd0ba`.
- Self-reviewed vs REQ-01..06 (all covered; REQ-06 = no existing skill modified).
- Dogfooded in unprimed session `a7c51c9d` on epic `25064508`: **plan-review PASSED first
  round** (commit `c74680f`).
- Post-dogfood fixes A, 1, B, 2, C applied (this commit) — detail in
  `dev/sessions/session-20260625-planning-skill-authoring.md` § "Dogfood outcome".

## Open / next steps (in order)
1. **Cold-start dogfood — recommended before completion.** REQ-01's core path (vague prompt
   → *created* container + criteria) is **untested**; the run exercised reconcile-existing
   only. Run a fresh **unprimed** session with a bare vague seed (no pre-existing epic).
   Verify it ingests the artifact, interviews to the `[hard]` DoD floor, and does **not**
   fabricate criteria. Fold any gaps into `eed6750c`.
2. **code-review gate:** `jit gate pass eed6750c code-review`; read the recorded status
   (`jit gate check eed6750c code-review`); resolve blocking findings; a recorded `passed`
   is terminal.
3. **Complete:** `jit issue update eed6750c --state done`; commit.

## Decisions / traps — do not repeat
- A recorded `plan-review` (or any gate) `passed` is **terminal** — do **not** re-run to
  "clean up" a minor/advisory note. The dogfood agent tried exactly this; the owner stopped
  it. Now encoded in Phase 7.
- The node's owner **runs its own gate** from within the task (supersedes the 2026-06-20
  design-doc §7 "leave PENDING for a standard runner").
- Keep the skill **domain-agnostic and terse**. A verbose, jit-specific invariant-check
  paragraph was rejected during fix C; phrase generically (no `graph.rs`/`CLAUDE.md`/`epic`
  literals in skill prose).
- `jit gate pass` exit codes are verdict-meaningful now (0=pass, 4=FAIL, 10=runner error);
  still prefer the recorded status over an outer wrapper's signal.
- The 10 design decisions + rejected options live in the authoring session doc — read it
  before changing skill behavior.

## Deferred follow-ups (NOT in eed6750c scope)
- Promote `content-standards.md` to a neutral shared SSOT (would modify jit-manage; REQ-06
  forbids it here).
- Fix 3 (express a foundation-reuse prerequisite as a dependency on `P`) — skipped by owner.
- Autonomous description-match triggering is untested (the dogfood used the `/jit-plan`
  slash command).

## Pointers
- Skill: `.claude/skills/jit-plan/`.
- Authoring + dogfood record: `dev/sessions/session-20260625-planning-skill-authoring.md`.
- Dogfood plan artifact: `dev/active/25064508-d563-4073-a970-296607a01adc-plan.md`.
- Dogfood transcript: `~/.claude/projects/-home-vkaskivuo-Projects-just-in-time/a7c51c9d-*.jsonl`.
