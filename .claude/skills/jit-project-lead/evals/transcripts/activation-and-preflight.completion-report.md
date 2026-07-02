# activation-and-preflight — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/activation` (setup scenario `two-tier`). Date: 2026-07-03.

## 1. Pre-flight

Ran all 5 steps in order, as specified in SKILL.md §Pre-flight.

1. **Verify `.jit/` exists** — present at repo root (contains `config.toml`, `templates.toml`,
   `rules.toml`, `gates.json`, `index.json`, `events.jsonl`, `issues/`, `schemas/`). Pass.
2. **Run `jit recover`** — exit 0. "Recovery complete: Stale locks cleaned: 1; Index rebuilt:
   true; Expired leases evicted: 0; Temp files removed: 0." Removed stale lock
   `.git/jit/locks/rules.lock`. The one permitted side effect. Pass.
3. **Read `.jit/config.toml`** — extracted `[type_hierarchy]` `types = { milestone=1, epic=2,
   story=3, planning=3, breakdown=3, task=4, bug=4, enhancement=4 }` (line 28),
   `strategic_types = ["milestone", "epic"]` (line 31), and `[documentation]`. Pass.
4. **Read `.jit/templates.toml`** — one `[[template]]` entry (`plan`) with `applies_to = ["epic"]`
   (line 12). Union of `applies_to` = `{ epic }`. Pass.
5. **Read canonical content standards** — resolved `../../../docs/reference/jit-content-standards.md`
   from the skill dir to `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`;
   readable (6994 bytes), read in full. Pass.

## 2. Tier derivation

Primary path (templates present, STRATEGIC LIST non-empty).

- **Steward anchor = `milestone`** — first entry of `strategic_types` (`config.toml:31`).
- **Delegation boundary = `{ epic }`** — union of `applies_to` (`templates.toml:12`).
- **Shape = two tier** — two strategic entries and anchor `milestone` not in boundary `{epic}`.

Assumption checks (LEVEL MAP `config.toml:28`): A `milestone`(1) ≤ `epic`(2) → holds; B `epic`(2)
≥ `milestone`(1) → holds. Matches tier-derivation.md worked verification Ruleset 1.

## 3. Stop point

Stopped at §Mode dispatch (stub). Reason given (SKILL.md:76-77): the four-mode routing block and
mode bodies are authored by the not-yet-landed four-mode front-door work; "Until that work lands,
complete pre-flight, report that mode routing is pending, and stop." No stop-and-escalate condition
and no tier-derivation stop triggered. Mode routing reported pending.

## 4. Command / state log

Commands under `/tmp/jit-plead-evals/activation`: `ls -la` / `ls -la .jit` (read-only);
`jit recover` (cleared 1 stale lock — permitted side effect); `jit issue list --json` (read-only
confirmation); `git status --porcelain` (read-only). `jit config show-hierarchy` NOT run (fallback-only
input, fallback not reached). Reads only: SKILL.md, references/tier-derivation.md, config.toml,
templates.toml, content-standards doc.

**Issue state:** no `.jit/` issue state created or mutated. `jit issue list --json` →
`{"count": 0, "issues": []}`; `events.jsonl` 0 bytes; no files under `.jit/issues/`. No issues, gates,
config, or templates created or modified. Only working-tree change is `jit recover`'s lock cleanup.
