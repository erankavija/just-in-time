# jit-project-lead — Eval Results

Adjudicated baseline for the scenarios in [`evals.json`](evals.json), graded under the
method in [`docs/reference/skill-eval-adjudication.md`](../../../../docs/reference/skill-eval-adjudication.md):
each scenario's `expected_output` is decomposed into an itemized checklist and every item is
scored against observable evidence from a recorded run.

**Scope:** the jit-project-lead **skeleton** — activation, pre-flight, and config-derived tier
derivation. The mode-routing block and mode bodies are out of scope (later stories own them); every
scenario runs to the skeleton's mode-dispatch stub, which completes pre-flight, derives tiers, reports
mode routing pending, and stops.

**Runner:** harness-governed fresh-context sub-agent given the scenario `prompt` verbatim (the
equivalent-runner path documented in the adjudication method), one per scenario, each against an
isolated repo built by [`setup-test-repo.sh`](setup-test-repo.sh) under `/tmp/jit-plead-evals/<scenario>`.
The `jit` CLI binary (`/home/vkaskivuo/.cargo/bin/jit`) was used directly; no MCP. Verdicts were scored by
inspecting the final repo state (`jit issue list --json`, `git status`, config/template contents) and the
run's completion report, not the run's self-report alone. Because the skeleton stops at the mode-dispatch
stub and never writes issue state, tier-derivation outputs leave no on-disk trace; for those items the
completion report is the evidence, cross-checked against the config/template inputs and the independently
re-verified repo state. Each scenario carrying a "no mutation / did not apply" negative item is backed by
the report's command / state log (adjudication method's equivalent-runner rule) **and** the
independently-observed empty issue set.

Independent post-run repo-state check (all four repos, 2026-07-03): `jit issue list --json` → `count: 0`;
`.jit/events.jsonl` = 0 bytes; `.jit/issues/` empty; `git status --porcelain` (tracked) clean, so no
config/template file was modified; `templates.toml` present for `activation`/`two-tier`/`collapsed` and
absent for `fallback`, matching each scenario's setup.

| Scenario | Date | Verdict | Backing |
|---|---|---|---|
| `activation-and-preflight` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/activation-and-preflight.completion-report.md) |
| `tier-derivation-two-tier` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/tier-derivation-two-tier.completion-report.md) |
| `tier-derivation-collapsed` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/tier-derivation-collapsed.completion-report.md) |
| `fallback-stop-and-ask` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/fallback-stop-and-ask.completion-report.md) |

---

## `activation-and-preflight` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/activation` (setup scenario `two-tier`).
- **Run report:** [`transcripts/activation-and-preflight.completion-report.md`](transcripts/activation-and-preflight.completion-report.md).

`expected_output`: *"The jit-project-lead skill activates and executes its pre-flight in order: confirms
.jit/ exists, runs `jit recover`, reads .jit/config.toml (type hierarchy and strategic_types), reads
.jit/templates.toml (the applies_to lists), and reads the canonical content-standards doc. It then performs
tier derivation once and, reaching the mode-dispatch stub, reports that mode routing is pending and stops
without authoring any mode behavior. No .jit/ issue state is created or mutated (only `jit recover`'s lock
cleanup is permitted)."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Skill activates and follows SKILL.md | Report §1 shows the agent read SKILL.md and executed its documented pre-flight sequence | PASS |
| 2 | Confirms `.jit/` exists | Report §1 step 1: `.jit/` present at repo root | PASS |
| 3 | Runs `jit recover` | Report §1 step 2: `jit recover` exit 0, cleaned 1 stale lock | PASS |
| 4 | Reads `.jit/config.toml` (types + strategic_types) | Report §1 step 3: extracted `types` map + `strategic_types = ["milestone","epic"]` | PASS |
| 5 | Reads `.jit/templates.toml` (applies_to) | Report §1 step 4: `applies_to = ["epic"]`, union `{epic}` | PASS |
| 6 | Reads canonical content-standards doc | Report §1 step 5: resolved and read `jit-content-standards.md` (6994 bytes) | PASS |
| 7 | Performs tier derivation once | Report §2: primary-path derivation anchor=milestone, boundary={epic}, two tier | PASS |
| 8 | Reaches the stub, reports mode routing pending, stops without authoring mode behavior | Report §3: stopped at §Mode dispatch (stub), mode routing reported pending, no mode body authored | PASS |
| 9 | No `.jit/` issue state created/mutated (recover lock cleanup only) | **Repo-state:** `jit issue list --json` → `count:0`; `events.jsonl` 0 bytes; `.jit/issues/` empty; tracked git clean. **Run-record:** report §4 command/state log lists only reads + `jit recover`, no issue-writing command. Both halves per the negative-item rule. | PASS |

**Verdict: PASS** (9/9).

---

## `tier-derivation-two-tier` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/two-tier` (setup scenario `two-tier`, this repository's own ruleset).
- **Run report:** [`transcripts/tier-derivation-two-tier.completion-report.md`](transcripts/tier-derivation-two-tier.completion-report.md).

`expected_output`: *"Tier derivation against this repo's ruleset yields steward anchor = milestone (the
first entry of strategic_types in .jit/config.toml), delegation boundary = {epic} (the union of applies_to
across the .jit/templates.toml [[template]] entries), and shape = two tier (two strategic entries and the
anchor is not itself a boundary type). Assumption checks A (strategic order) and B (boundary at or below
anchor) pass. The run reports these derived tiers and stops at the mode-dispatch stub."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Steward anchor = `milestone` (first strategic_types entry) | Report §2: anchor `milestone` cited to `config.toml:31`; input independently confirmed (`strategic_types = ["milestone","epic"]`) | PASS |
| 2 | Delegation boundary = `{epic}` (union of applies_to) | Report §2: boundary `{epic}` cited to `templates.toml:12`; input independently confirmed (`applies_to = ["epic"]`) | PASS |
| 3 | Shape = two tier (two strategic entries, anchor not a boundary type) | Report §2: two entries, `milestone` ∉ `{epic}` → two tier | PASS |
| 4 | Assumption check A passes | Report §2: milestone(1) ≤ epic(2), anchor holds minimum → PASS | PASS |
| 5 | Assumption check B passes | Report §2: epic(2) ≥ milestone(1) → PASS | PASS |
| 6 | Reports derived tiers and stops at mode-dispatch stub | Report §3: tiers reported, stopped at §Mode dispatch (stub), mode routing pending | PASS |

Also observed: `jit issue list --json` → `count:0`; tracked git clean (config/templates untouched).

**Verdict: PASS** (6/6).

---

## `tier-derivation-collapsed` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/collapsed` (setup scenario `collapsed`, the `docs/examples/research` ruleset).
- **Run report:** [`transcripts/tier-derivation-collapsed.completion-report.md`](transcripts/tier-derivation-collapsed.completion-report.md).

`expected_output`: *"Tier derivation against the research example ruleset yields steward anchor = goal (the
sole entry of strategic_types in .jit/config.toml), delegation boundary = {goal} (the union of applies_to
across .jit/templates.toml), and shape = collapsed single tier (exactly one strategic entry that is itself a
member of the boundary set). Assumption checks A and B pass. The run reports these derived tiers and stops at
the mode-dispatch stub."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Steward anchor = `goal` (sole strategic_types entry) | Report §2: anchor `goal` cited to `config.toml:34`; input independently confirmed (`strategic_types = ["goal"]`) | PASS |
| 2 | Delegation boundary = `{goal}` (union of applies_to) | Report §2: boundary `{goal}` cited to `templates.toml:18` (`applies_to = ["goal"]`) | PASS |
| 3 | Shape = collapsed single tier (one strategic entry that is itself a boundary type) | Report §2: single entry `goal` ∈ boundary `{goal}` → collapsed single tier | PASS |
| 4 | Assumption check A passes | Report §2: single entry trivially ordered; anchor `goal`(2) holds minimum → PASS | PASS |
| 5 | Assumption check B passes | Report §2: boundary `goal`(2) ≥ anchor `goal`(2) → PASS | PASS |
| 6 | Reports derived tiers and stops at mode-dispatch stub | Report §3: tiers reported, stopped at §Mode dispatch (stub), mode routing pending | PASS |

Also observed: `jit issue list --json` → `count:0`; tracked git clean (config/templates untouched). Matches
tier-derivation.md worked verification Ruleset 2.

**Verdict: PASS** (6/6).

---

## `fallback-stop-and-ask` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/fallback` (setup scenario `fallback`: bare `jit init`, no `.jit/templates.toml`).
- **Run report:** [`transcripts/fallback-stop-and-ask.completion-report.md`](transcripts/fallback-stop-and-ask.completion-report.md).

`expected_output`: *"With .jit/templates.toml absent, tier derivation routes to the numeric-level fallback
rather than the primary path. The run reads the level map from `jit config show-hierarchy --json`, recovers a
candidate proposal (anchor = milestone from strategic_types; boundary = {epic}, the type at the next distinct
level below the anchor), and then STOPS and asks the invoker to confirm, explicitly naming .jit/templates.toml
as the missing input and reporting what show-hierarchy returned. It does NOT apply the recovered tiers
unconfirmed and does not proceed past derivation: no .jit/ issue state is created or mutated and the
config/templates are left as-is."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Derivation routes to the fallback (not the primary path) | Report §2: guard fired, fallback taken; independently confirmed `.jit/templates.toml` absent in repo | PASS |
| 2 | Reads the level map from `jit config show-hierarchy --json` | Report §3: LEVEL MAP `{milestone:1, epic:2, story:3, task:4}`, `message` key dropped | PASS |
| 3 | Recovers anchor = `milestone` from strategic_types | Report §3: fallback step 2, anchor = first strategic_types entry `milestone` | PASS |
| 4 | Recovers boundary = `{epic}` (next distinct level below anchor) | Report §3: fallback step 3, level below milestone(1) is 2 → sole member `epic` | PASS |
| 5 | STOPS and asks the invoker to confirm | Report §4: stopped at Fallback step 5, "Stop with a proposal", confirm-before-applying ask | PASS |
| 6 | Explicitly names `.jit/templates.toml` as the missing input and reports show-hierarchy output | Report §2 names the absent `templates.toml` as the sole trigger; §3 reports the show-hierarchy LEVEL MAP | PASS |
| 7 | Does NOT apply recovered tiers and does not proceed past derivation | **Run-record:** report §4 explicitly states tiers not applied and derivation not passed; §5 command log shows only reads + `jit recover`. **Repo-state:** `jit issue list --json` → `count:0`; `events.jsonl` 0 bytes; tracked git clean (config/templates untouched, `templates.toml` still absent). Both halves per the negative-item rule. | PASS |

**Verdict: PASS** (7/7).

---

## Reproducing a verdict

1. Rebuild the scenario's repo: `bash setup-test-repo.sh /tmp/jit-plead-evals/<scenario> <setup_scenario>`
   (setup scenarios: `two-tier`, `collapsed`, `fallback`).
2. Run the scenario `prompt` (from `evals.json`) with an equivalent runner, substituting the scratch repo
   path for `{REPO_PATH}`.
3. Re-score the checklist above against the run's completion report and the final repo state
   (`jit issue list --json`, `git status`, config/template contents).

See [`docs/reference/skill-eval-adjudication.md`](../../../../docs/reference/skill-eval-adjudication.md)
for the full method.
