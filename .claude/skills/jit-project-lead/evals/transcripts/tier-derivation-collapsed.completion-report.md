# tier-derivation-collapsed — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/collapsed` (setup scenario `collapsed`, the research example ruleset).
Date: 2026-07-03.

## 1. Pre-flight

1. **`.jit/` exists** at repo root. Confirmed.
2. **`jit recover`** exit 0: 1 stale lock cleaned (`.git/jit/locks/rules.lock`), index and claims index
   rebuilt, 0 leases, 0 temp files. Lock/index hygiene only.
3. **Read `.jit/config.toml`**: `[type_hierarchy].types = { goal=2, experiment=3, planning=3,
   breakdown=3 }` (line 31), `strategic_types = ["goal"]` (line 34). Research-program example ruleset.
4. **Read `.jit/templates.toml`**: one `[[template]]` (`plan`) with `applies_to = ["goal"]` (line 18).
5. **Read content standards** at `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`
   (readable). Held in memory.

## 2. Tier derivation

Inputs: STRATEGIC LIST = `["goal"]` (`config.toml:34`); BOUNDARY SET = union of `applies_to` = `{ goal }`
(`templates.toml:18`); LEVEL MAP = `{ goal:2, experiment:3, planning:3, breakdown:3 }` (`config.toml:31`,
confirmed via `jit config show-hierarchy --json`). Guard: templates present and STRATEGIC LIST non-empty
→ **primary path**.

- **Steward ANCHOR = `goal`** — first entry of `strategic_types` (`config.toml:34`).
- **Delegation BOUNDARY = `goal`** — union of every `applies_to`; here just `plan.applies_to = ["goal"]`
  (`templates.toml:18`).
- **SHAPE = collapsed single tier** — STRATEGIC LIST has exactly one entry and that entry (`goal`) is a
  member of BOUNDARY SET; the anchor is itself the breakable container (procedure step 5, bullet 1).
- **Assumption A: PASS** — single entry, trivially ordered; anchor `goal`(2) holds the minimum level.
- **Assumption B: PASS** — the only boundary type `goal`(2) ≥ anchor `goal`(2).

Matches tier-derivation.md worked verification "Ruleset 2 — research example (collapsed single tier)".

## 3. Stop point

Stopped at §Mode dispatch. After pre-flight and tier derivation, the scenario prompt carries no explicit
mode signal, so the front door cannot pick exactly one of the four modes: per `references/mode-routing.md`
Stop and ask, it reports the derived tiers and asks which mode to run rather than guessing one. Not a
tier-derivation stop-and-ask (derivation succeeded on the primary path).

## 4. Command / state log

Under `/tmp/jit-plead-evals/collapsed`: `ls -la`, `git status`, `ls -la .jit/` (read-only); `jit recover`
(lock/index hygiene, no issue write); `jit config show-hierarchy --json` (read-only). Reads of SKILL.md,
references/tier-derivation.md, config.toml, templates.toml, content-standards doc.

**No `.jit/` issue state created or mutated.** No issues, gates, config, or templates created or modified.
`.jit/issues/` remained empty throughout; only `jit recover` lock/index cleanup occurred.
