# jit-project-lead — Eval Results

Adjudicated baseline for the scenarios in [`evals.json`](evals.json), graded under the
method in [`dev/eval/skill-eval-adjudication.md`](../../../../dev/eval/skill-eval-adjudication.md):
each scenario's `expected_output` is decomposed into an itemized checklist and every item is
scored against observable evidence from a recorded run.

**Scope:** two scenario families.

- **Skeleton evals (`activation-and-preflight`, `tier-derivation-two-tier`, `tier-derivation-collapsed`,
  `fallback-stop-and-ask`)** cover activation, pre-flight, and config-derived tier derivation. They do not
  exercise a specific mode body. Their prompts are generic with no explicit mode signal, so the front door
  completes pre-flight, derives tiers, then stops and asks which of the four modes to run (per
  `references/mode-routing.md` Stop and ask) — it does not guess a mode. The `fallback-stop-and-ask`
  scenario stops earlier, at tier derivation, before routing is reached.
- **Routing evals (`routing-mode-1` … `routing-mode-4`)** provide the REQ-01 verification issue `6c5f70ad`
  requires: given a representative opening request for EACH of the four modes (the four requests in
  `references/mode-routing.md`), the front door classifies it into the CORRECT mode and begins that mode's
  handoff, **halting at the handoff boundary** — it does not run the full mode body (no execution lead
  dispatched, no planning interview, no sweep). They run against the `routing` setup scenario, which seeds
  the this-repo two-tier ruleset plus one existing anchor-tier (`type:milestone`) container "Demo release
  readiness" with two `type:epic` children carrying its `milestone:demo-release` membership label, so mode 1
  has a real container to resolve; modes 2/3/4 ignore it. All four routed to the correct mode with no
  misroute (1→1, 2→2, 3→3, 4→4).

**Runner:** harness-governed fresh-context sub-agent given the scenario `prompt` verbatim (the
equivalent-runner path documented in the adjudication method), one per scenario, each against an
isolated repo built by [`setup-test-repo.sh`](setup-test-repo.sh) under `/tmp/jit-plead-evals/<scenario>`.
The `jit` CLI binary (`/home/vkaskivuo/.cargo/bin/jit`) was used directly; no MCP. Verdicts were scored by
inspecting the final repo state (`jit issue list --json`, `git status`, config/template contents) and the
run's completion report, not the run's self-report alone. Because each run stops at the front-door
stop-and-ask (or, for `fallback`, at tier derivation) and never writes issue state, tier-derivation
outputs leave no on-disk trace; for those items the
completion report is the evidence, cross-checked against the config/template inputs and the independently
re-verified repo state. Each scenario carrying a "no mutation / did not apply" negative item is backed by
the report's command / state log (adjudication method's equivalent-runner rule) **and** the
independently-observed empty issue set.

Independent post-run repo-state check (skeleton repos, 2026-07-03): `jit issue list --json` → `count: 0`;
`.jit/events.jsonl` = 0 bytes; `.jit/issues/` empty; `git status --porcelain` (tracked) clean, so no
config/template file was modified; `templates.toml` present for `activation`/`two-tier`/`collapsed` and
absent for `fallback`, matching each scenario's setup. For the four `routing` repos the seed leaves a
non-empty baseline, so the negative check compares against it: pre-run and post-run each show `jit issue
list --json` → `count: 3`, `.jit/events.jsonl` = 1275 bytes, and exactly one `type:milestone` container —
unchanged by the run, so no mode body executed and no issue state was written.

| Scenario | Date | Verdict | Backing |
|---|---|---|---|
| `activation-and-preflight` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/activation-and-preflight.completion-report.md) |
| `tier-derivation-two-tier` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/tier-derivation-two-tier.completion-report.md) |
| `tier-derivation-collapsed` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/tier-derivation-collapsed.completion-report.md) |
| `fallback-stop-and-ask` | 2026-07-03 | **PASS** | itemized checklist below + [run report](transcripts/fallback-stop-and-ask.completion-report.md) |
| `routing-mode-1` | 2026-07-03 | **PASS** (→ mode 1) | itemized checklist below + [run report](transcripts/routing-mode-1.completion-report.md) |
| `routing-mode-2` | 2026-07-03 | **PASS** (→ mode 2) | itemized checklist below + [run report](transcripts/routing-mode-2.completion-report.md) |
| `routing-mode-3` | 2026-07-03 | **PASS** (→ mode 3) | itemized checklist below + [run report](transcripts/routing-mode-3.completion-report.md) |
| `routing-mode-4` | 2026-07-03 | **PASS** (→ mode 4) | itemized checklist below + [run report](transcripts/routing-mode-4.completion-report.md) |

---

## `activation-and-preflight` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/activation` (setup scenario `two-tier`).
- **Run report:** [`transcripts/activation-and-preflight.completion-report.md`](transcripts/activation-and-preflight.completion-report.md).

`expected_output`: *"The jit-project-lead skill activates and executes its pre-flight in order: confirms
.jit/ exists, runs `jit recover`, reads .jit/config.toml (type hierarchy and strategic_types), reads
.jit/templates.toml (the applies_to lists), and reads the canonical content-standards doc. It then performs
tier derivation once and, reaching mode dispatch with a generic prompt that carries no explicit mode signal,
stops and asks which of the four modes to run rather than guessing one, without authoring any mode behavior.
No .jit/ issue state is created or mutated (only `jit recover`'s lock cleanup is permitted)."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Skill activates and follows SKILL.md | Report §1 shows the agent read SKILL.md and executed its documented pre-flight sequence | PASS |
| 2 | Confirms `.jit/` exists | Report §1 step 1: `.jit/` present at repo root | PASS |
| 3 | Runs `jit recover` | Report §1 step 2: `jit recover` exit 0, cleaned 1 stale lock | PASS |
| 4 | Reads `.jit/config.toml` (types + strategic_types) | Report §1 step 3: extracted `types` map + `strategic_types = ["milestone","epic"]` | PASS |
| 5 | Reads `.jit/templates.toml` (applies_to) | Report §1 step 4: `applies_to = ["epic"]`, union `{epic}` | PASS |
| 6 | Reads canonical content-standards doc | Report §1 step 5: resolved and read `jit-content-standards.md` (6994 bytes) | PASS |
| 7 | Performs tier derivation once | Report §2: primary-path derivation anchor=milestone, boundary={epic}, two tier | PASS |
| 8 | Reaches mode dispatch, stops and asks which mode (no explicit signal), no mode behavior authored | Report §3: stopped at §Mode dispatch, generic prompt has no mode signal → asked which of the four modes, no mode body authored | PASS |
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
anchor) pass. The run reports these derived tiers, then — the prompt carrying no explicit mode signal —
stops and asks which of the four modes to run rather than guessing one."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Steward anchor = `milestone` (first strategic_types entry) | Report §2: anchor `milestone` cited to `config.toml:31`; input independently confirmed (`strategic_types = ["milestone","epic"]`) | PASS |
| 2 | Delegation boundary = `{epic}` (union of applies_to) | Report §2: boundary `{epic}` cited to `templates.toml:12`; input independently confirmed (`applies_to = ["epic"]`) | PASS |
| 3 | Shape = two tier (two strategic entries, anchor not a boundary type) | Report §2: two entries, `milestone` ∉ `{epic}` → two tier | PASS |
| 4 | Assumption check A passes | Report §2: milestone(1) ≤ epic(2), anchor holds minimum → PASS | PASS |
| 5 | Assumption check B passes | Report §2: epic(2) ≥ milestone(1) → PASS | PASS |
| 6 | Reports derived tiers, then stops and asks which mode (no explicit signal) | Report §3: tiers reported, stopped at §Mode dispatch, generic prompt has no mode signal → asked which of the four modes | PASS |

Also observed: `jit issue list --json` → `count:0`; tracked git clean (config/templates untouched).

**Verdict: PASS** (6/6).

---

## `tier-derivation-collapsed` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/collapsed` (setup scenario `collapsed`, the `docs/examples/research` ruleset).
- **Run report:** [`transcripts/tier-derivation-collapsed.completion-report.md`](transcripts/tier-derivation-collapsed.completion-report.md).

`expected_output`: *"Tier derivation against the research example ruleset yields steward anchor = goal (the
sole entry of strategic_types in .jit/config.toml), delegation boundary = {goal} (the union of applies_to
across .jit/templates.toml), and shape = collapsed single tier (exactly one strategic entry that is itself a
member of the boundary set). Assumption checks A and B pass. The run reports these derived tiers, then — the
prompt carrying no explicit mode signal — stops and asks which of the four modes to run rather than guessing
one."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Steward anchor = `goal` (sole strategic_types entry) | Report §2: anchor `goal` cited to `config.toml:34`; input independently confirmed (`strategic_types = ["goal"]`) | PASS |
| 2 | Delegation boundary = `{goal}` (union of applies_to) | Report §2: boundary `{goal}` cited to `templates.toml:18` (`applies_to = ["goal"]`) | PASS |
| 3 | Shape = collapsed single tier (one strategic entry that is itself a boundary type) | Report §2: single entry `goal` ∈ boundary `{goal}` → collapsed single tier | PASS |
| 4 | Assumption check A passes | Report §2: single entry trivially ordered; anchor `goal`(2) holds minimum → PASS | PASS |
| 5 | Assumption check B passes | Report §2: boundary `goal`(2) ≥ anchor `goal`(2) → PASS | PASS |
| 6 | Reports derived tiers, then stops and asks which mode (no explicit signal) | Report §3: tiers reported, stopped at §Mode dispatch, generic prompt has no mode signal → asked which of the four modes | PASS |

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

## `routing-mode-1` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/routing-mode-1` (setup scenario `routing`, seed milestone `03b5911f`).
- **Run report:** [`transcripts/routing-mode-1.completion-report.md`](transcripts/routing-mode-1.completion-report.md).
- **Opening request:** *"Drive milestone 03b5911f to done — steward it across its epics."*

`expected_output`: *"The skill completes pre-flight and tier derivation (anchor = milestone, boundary =
{epic}, two tier), then classifies the opening request as MODE 1 (lead an already-existing strategic
container): the request names an id that resolves to one existing anchor-tier container. It begins mode 1's
resolve-and-handoff: resolves the seed container by id via `jit issue show`, confirms its type equals the
ANCHOR TYPE (milestone) so it is at the steward anchor tier, and hands it to `## Sub-strategic dispatch` —
naming the resolved container. It HALTS at that handoff boundary: it does NOT enter the dispatch loop, does
NOT dispatch any jit-execution-lead, and does not run waves or coherence review. It routes to mode 1 only,
with no misroute to modes 2/3/4. No .jit/ issue state is created or mutated (only `jit recover`'s lock
cleanup is permitted)."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Completes pre-flight + tier derivation (anchor=milestone, boundary={epic}, two tier) | Report §1–§2: primary-path derivation anchor `milestone`, boundary `{epic}`, two tier, assumption checks A/B pass | PASS |
| 2 | Classifies the request as MODE 1 | Report §3: routed to Mode 1, id `03b5911f` resolves to one anchor-tier container | PASS |
| 3 | Resolves the seed container by id via `jit issue show` | Report §4: `jit issue show 03b5911f` → "Demo release readiness"; independently confirmed the seed milestone exists (`type:milestone`, `milestone:demo-release`) | PASS |
| 4 | Confirms resolved type equals ANCHOR TYPE (milestone), i.e. anchor tier | Report §4: type `milestone` == ANCHOR TYPE `milestone` → is the steward anchor tier | PASS |
| 5 | Hands it to `## Sub-strategic dispatch`, naming the container | Report §4: handed container id + invocation context to `## Sub-strategic dispatch` | PASS |
| 6 | HALTS at handoff boundary — no dispatch loop, no execution lead, no waves/coherence review | Report §4: explicitly halted; did not enter dispatch loop, layer waves, or dispatch any `jit-execution-lead`. **Run-record:** §5 command log lists only reads + `jit recover` + `jit issue show`, no dispatch/worktree command | PASS |
| 7 | Routes to mode 1 only — no misroute to 2/3/4 | Report §3: explicit not-Mode-2/3/4 rationale; exactly one mode signalled | PASS |
| 8 | No `.jit/` issue state created/mutated (recover lock cleanup only) | **Run-record:** §5 affirms no issue write. **Repo-state:** post-run `count:3`, `events.jsonl` 1275 bytes (= seed baseline), 1 milestone — unchanged from pre-run seed. Both halves per the negative-item rule. | PASS |

**Verdict: PASS** (8/8) — routed to **mode 1** (correct).

---

## `routing-mode-2` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/routing-mode-2` (setup scenario `routing`).
- **Run report:** [`transcripts/routing-mode-2.completion-report.md`](transcripts/routing-mode-2.completion-report.md).
- **Opening request:** *"Stand up multi-tenant support for the product — figure out what that means and get it delivered."*

`expected_output`: *"The skill completes pre-flight and tier derivation (anchor = milestone, boundary =
{epic}, two tier), then classifies the opening request as MODE 2 (plan and execute a vague high-level goal):
a cold-start goal with no container yet, asking to both scope and deliver. It begins mode 2's handoff by
routing to the jit-planning-lead skill at the strategic altitude (its cold-start interview / research-and-plan
entry), supplying the goal and the anchor tier. It HALTS at that handoff boundary: it does NOT reimplement or
run the planning interview here, and dispatches NO worker/execution lead from the front door. It routes to
mode 2 only, with no misroute (not mode 1: no existing container is named; not mode 3: delivery is asked, not
deliberation only; not mode 4: not a standards audit). No .jit/ issue state is created or mutated (only `jit
recover`'s lock cleanup is permitted)."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Completes pre-flight + tier derivation (anchor=milestone, boundary={epic}, two tier) | Report §1–§2: primary-path derivation anchor `milestone`, boundary `{epic}`, two tier, checks A/B pass | PASS |
| 2 | Classifies the request as MODE 2 | Report §3: routed to Mode 2 — a goal with no resolvable container, asking to scope ("figure out what that means") and deliver ("get it delivered") | PASS |
| 3 | Begins mode 2 handoff to jit-planning-lead at the strategic altitude (cold-start entry), supplying goal + anchor tier | Report §4: routes to `jit-planning-lead` at the `milestone` anchor tier, cold-start / research-and-plan (interview) entry, supplying goal + anchor tier | PASS |
| 4 | No worker/execution lead dispatched from the front door | Report §4: confirmed no worker dispatched | PASS |
| 5 | HALTS at handoff boundary — does not reimplement/run the planning interview | Report §4: halted; did not enter or reimplement the interview or create a container. **Run-record:** §5 log lists only reads + `jit recover` + `jit issue list`, no planning/dispatch command | PASS |
| 6 | Routes to mode 2 only — no misroute (not 1: no container named; not 3: delivery asked; not 4: not audit) | Report §3: itemized not-Mode-1/3/4 rationale; exactly one mode signalled | PASS |
| 7 | No `.jit/` issue state created/mutated (recover lock cleanup only) | **Run-record:** §5 affirms no issue write. **Repo-state:** post-run `count:3`, `events.jsonl` 1275 bytes, 1 milestone — unchanged from seed. Both halves per the negative-item rule. | PASS |

**Verdict: PASS** (7/7) — routed to **mode 2** (correct).

---

## `routing-mode-3` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/routing-mode-3` (setup scenario `routing`).
- **Run report:** [`transcripts/routing-mode-3.completion-report.md`](transcripts/routing-mode-3.completion-report.md).
- **Opening request:** *"Let's work through the direction for next quarter and settle the vision before we commit anyone to it."*

`expected_output`: *"The skill completes pre-flight and tier derivation (anchor = milestone, boundary =
{epic}, two tier), then classifies the opening request as MODE 3 (steering discussion): interactive
vision/decision work with no workers dispatched — deliberation, not delivery. It begins mode 3's handoff by
routing to the jit-planning-lead skill at the strategic altitude (its interactive strategic-altitude
planning), with no execution committed. It HALTS at that handoff boundary: it does NOT reimplement planning
here and dispatches NO worker/execution lead from the front door. It routes to mode 3 only, with no misroute
(not mode 2: no delivery/build is committed, deliberation only; not mode 1: no existing container is named;
not mode 4: not a standards audit). No .jit/ issue state is created or mutated (only `jit recover`'s lock
cleanup is permitted)."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Completes pre-flight + tier derivation (anchor=milestone, boundary={epic}, two tier) | Report §1–§2: primary-path derivation anchor `milestone`, boundary `{epic}`, two tier, checks A/B pass | PASS |
| 2 | Classifies the request as MODE 3 | Report §3: routed to Mode 3 — "work through the direction … settle the vision" (deliberation) with "before we commit anyone to it" (no dispatch) | PASS |
| 3 | Begins mode 3 handoff to jit-planning-lead at the strategic altitude (interactive planning), no execution committed | Report §4: routes to `jit-planning-lead` at the `milestone` anchor tier, interactive strategic-altitude planning | PASS |
| 4 | No worker/execution lead dispatched from the front door | Report §4: confirmed no worker dispatched | PASS |
| 5 | HALTS at handoff boundary — does not reimplement planning | Report §4: halted; did not follow planning-lead's body or run any planning. **Run-record:** §5 log lists only `jit recover` + reads | PASS |
| 6 | Routes to mode 3 only — no misroute (key split: not mode 2, delivery not committed) | Report §3: explicit Mode-2 disambiguation ("before we commit anyone to it" = refusal to dispatch → deliberation without delivery), plus not-Mode-1/4 rationale | PASS |
| 7 | No `.jit/` issue state created/mutated (recover lock cleanup only) | **Run-record:** §5 affirms no issue write. **Repo-state:** post-run `count:3`, `events.jsonl` 1275 bytes, 1 milestone — unchanged from seed. Both halves per the negative-item rule. | PASS |

**Verdict: PASS** (7/7) — routed to **mode 3** (correct).

---

## `routing-mode-4` — PASS (2026-07-03)

- **Repo:** `/tmp/jit-plead-evals/routing-mode-4` (setup scenario `routing`).
- **Run report:** [`transcripts/routing-mode-4.completion-report.md`](transcripts/routing-mode-4.completion-report.md).
- **Opening request:** *"Sweep the whole project for content-standards violations, auto-fix the mechanical ones, and list what needs a decision."*

`expected_output`: *"The skill completes pre-flight and tier derivation (anchor = milestone, boundary =
{epic}, two tier), then classifies the opening request as MODE 4 (standards sweep): a project-wide
content-standards audit that auto-fixes mechanical violations and surfaces the judgment calls. It begins mode
4's handoff by entering the `## Standards sweep mode` / `references/standards-sweep.md`. It HALTS at that
handoff boundary: it does NOT run the scanner or fixer scripts and does not produce the sweep report. It
routes to mode 4 only, with no misroute (not mode 1: no container to lead is named; not modes 2/3: no goal to
plan and no deliberation requested). No .jit/ issue state is created or mutated (only `jit recover`'s lock
cleanup is permitted)."*

| # | Item | Evidence | Mark |
|---|---|---|---|
| 1 | Completes pre-flight + tier derivation (anchor=milestone, boundary={epic}, two tier) | Report §1–§2: primary-path derivation anchor `milestone`, boundary `{epic}`, two tier, checks A/B pass | PASS |
| 2 | Classifies the request as MODE 4 | Report §3: routed to Mode 4 — a project-wide content-standards audit (auto-fix mechanical + surface judgment calls) | PASS |
| 3 | Begins mode 4 handoff by entering `## Standards sweep mode` / `references/standards-sweep.md` | Report §4: entered the `## Standards sweep mode` section (Mode 4 body) | PASS |
| 4 | HALTS at handoff boundary — does NOT run scanner/fixer scripts or produce the sweep report | Report §4: halted; `scripts/standards-scan.sh` and `scripts/standards-fix.sh` NOT run, no report produced. **Run-record:** §5 log lists only `jit --version` + `jit recover` + reads, no script invocation | PASS |
| 5 | Routes to mode 4 only — no misroute (not 1: no container; not 2/3: no goal/deliberation) | Report §3: itemized not-Mode-1/2/3 rationale; exactly one mode signalled | PASS |
| 6 | No `.jit/` issue state created/mutated (recover lock cleanup only) | **Run-record:** §5 affirms no write command. **Repo-state:** post-run `count:3`, `events.jsonl` 1275 bytes, 1 milestone — unchanged from seed. Both halves per the negative-item rule. | PASS |

**Verdict: PASS** (6/6) — routed to **mode 4** (correct).

---

## Reproducing a verdict

1. Rebuild the scenario's repo: `bash setup-test-repo.sh /tmp/jit-plead-evals/<scenario> <setup_scenario>`
   (setup scenarios: `two-tier`, `collapsed`, `fallback`, `routing`). The `routing` setup prints
   `MILESTONE_ID=<short_id>` on its last line.
2. Run the scenario `prompt` (from `evals.json`) with an equivalent runner, substituting the scratch repo
   path for `{REPO_PATH}`. For `routing-mode-1`, also substitute the printed `MILESTONE_ID` for the
   `{CONTAINER_ID}` placeholder (modes 2/3/4 carry no placeholder — their requests name no container).
3. Re-score the checklist above against the run's completion report and the final repo state
   (`jit issue list --json`, `git status`, config/template contents). For `routing` repos the seed leaves a
   non-empty baseline (`count:3`, `events.jsonl` 1275 bytes, one `type:milestone`); the negative "no
   mutation" items check the post-run state is unchanged from that baseline, not empty.

See [`dev/eval/skill-eval-adjudication.md`](../../../../dev/eval/skill-eval-adjudication.md)
for the full method.
