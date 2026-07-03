# routing-mode-1 — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/routing-mode-1` (setup scenario `routing`, seed milestone `03b5911f`
"Demo release readiness"). Opening request: *"Drive milestone 03b5911f to done — steward it
across its epics."* Date: 2026-07-03.

## 1. Pre-flight

1. **`.jit/` exists** at repo root. Pass.
2. **`jit recover`** exit 0: cleaned 1 stale lock (`.git/jit/locks/rules.lock`), rebuilt index and
   claims index. Permitted lock cleanup, no issue-state mutation.
3. **Read `.jit/config.toml`**: `strategic_types = ["milestone", "epic"]`; `types` map;
   `[documentation]` (development_root `dev`, permanent_paths `["docs/"]`).
4. **Read `.jit/templates.toml`**: one `[[template]]` (`plan`) with `applies_to = ["epic"]`.
5. **Read content standards** at `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`
   (readable). Pass.
6. **Resume read-back**: container resolved (see §4); its `documents` array is empty and no
   vision/progress artifacts exist (first-invocation state), so nothing to resume. Creating those
   artifacts belongs to the dispatch section past the halt boundary, so not performed.

## 2. Tier derivation

Primary path (both config inputs present).

- Inputs: STRATEGIC LIST = `["milestone","epic"]` (`config.toml:31`); BOUNDARY SET = union of
  `applies_to` = `{epic}` (`templates.toml:12`); LEVEL MAP = `{milestone:1, epic:2, story:3,
  planning:3, breakdown:3, task:4, bug:4, enhancement:4}` (`config.toml:28`).
- **ANCHOR TYPE = `milestone`** — first entry of STRATEGIC LIST.
- **BOUNDARY SET = `{epic}`**.
- **SHAPE = two tier** — two entries and anchor `milestone` ∉ BOUNDARY SET.
- Assumption A (strategic order): `milestone(1) ≤ epic(2)` holds. Assumption B (boundary at/below
  anchor): `epic(2) ≥ milestone(1)` holds.

## 3. Mode routing

**Routed to MODE 1** (lead an already-existing strategic container).

Triggering signal: the request "Drive **milestone `03b5911f`** to done — steward it across its
epics." gives a drive/steward-to-done imperative plus an explicit issue id (`03b5911f`) that
resolves to exactly one existing container at the ANCHOR TYPE (`milestone`) — Mode 1's defining
signal, a resolvable existing container at the anchor tier.

Did NOT match the other three:
- **Not Mode 2** — the container already exists (id resolves); no cold-start vague goal to scope.
- **Not Mode 3** — the request commits to delivery ("drive to done", "steward across its epics"),
  not deliberation with no dispatch.
- **Not Mode 4** — no project-wide content-standards audit; the request targets one named container.

Exactly one mode signalled → no stop-and-ask.

## 4. Handoff begun (Mode 1)

- **Resolved container** via `jit issue show 03b5911f`: id `03b5911f-2b29-440d-b801-0b2f469b2e3a`,
  short_id `03b5911f`, title "Demo release readiness". Resolves to exactly one issue.
- **Type**: `milestone` (labels `type:milestone`, `milestone:demo-release`).
- **Equals ANCHOR TYPE?** Yes — `milestone` == ANCHOR TYPE `milestone`, so it **is the steward
  anchor tier**. (Its two dependencies are `type:epic` — the sub-strategic children a lead owns.)
- **Section handed to**: `## Sub-strategic dispatch`, supplying the resolved container id plus the
  invocation context (the opening request and its "drive to done / steward across its epics"
  constraint).

**HALTED at the handoff boundary.** Did NOT enter the dispatch loop, layer waves, resolve/create
vision or progress artifacts, or dispatch any `jit-execution-lead`.

## 5. Command / state log

Under `/tmp/jit-plead-evals/routing-mode-1`: `ls -la .jit/` (read-only); `jit recover` (lock/index
cleanup); `jit issue show 03b5911f --json` (read-only container resolution). Plus read-only file
reads of `config.toml`, `templates.toml`, and the content-standards doc.

**No `.jit/` issue state created or mutated.** No issue added, updated, transitioned, labelled,
doc-linked, or gated. Only write: `jit recover`'s stale-lock cleanup (permitted).
