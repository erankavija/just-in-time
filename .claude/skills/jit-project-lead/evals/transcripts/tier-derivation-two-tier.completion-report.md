# tier-derivation-two-tier — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/two-tier` (setup scenario `two-tier`). Date: 2026-07-03.

## 1. Pre-flight

- **`.jit/` exists** at repo root. Pass.
- **`jit recover`** succeeded: cleaned 1 stale lock (`.git/jit/locks/rules.lock`), rebuilt index and
  claims index; 0 leases evicted, 0 temp files. Continue.
- **Read `.jit/config.toml`**: `strategic_types = ["milestone", "epic"]` (line 31); `types` map
  (line 28); `[documentation]`.
- **Read `.jit/templates.toml`**: one `[[template]]` (`plan`) with `applies_to = ["epic"]` (line 12).
- **Read content standards** at `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`
  (readable). Pass.

## 2. Tier derivation

- **Steward ANCHOR: `milestone`** — first entry of `strategic_types` (`.jit/config.toml:31`).
- **Delegation BOUNDARY: `{epic}`** — union of every `[[template]].applies_to`; only `plan` has
  `applies_to = ["epic"]` (`.jit/templates.toml:12`).
- **SHAPE: two tier** — STRATEGIC LIST has exactly two entries and anchor `milestone` ∉ boundary `{epic}`.
- **Path:** primary (both config inputs present). `jit config show-hierarchy --json` run only to confirm
  the LEVEL MAP (`milestone:1, epic:2, story/planning/breakdown:3, task/bug/enhancement:4`), matching
  `config.toml:28`.
- **Assumption A (strategic order): PASS** — milestone(1) ≤ epic(2); anchor holds the minimum level.
- **Assumption B (boundary at or below anchor): PASS** — epic(2) ≥ milestone(1).

## 3. Stop point

Stopped at §Mode dispatch. The scenario prompt is generic (no explicit mode signal), so the front door
cannot pick exactly one of the four modes: per `references/mode-routing.md` Stop and ask, it reports the
derived tiers and asks which mode to run rather than guessing one. Not a stop-and-escalate tooling
failure (none triggered).

## 4. Command / state log

Under `/tmp/jit-plead-evals/two-tier`: `ls -la .jit/` (read-only); `jit recover` (lock/index cleanup
only); reads of config.toml / templates.toml; `jit config show-hierarchy --json` (read-only).

**No `.jit/` issue state created or mutated.** No issues, gates, config, or templates created or
modified. Only state touched: `jit recover`'s lock/index cleanup.
