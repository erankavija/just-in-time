# routing-mode-3 — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/routing-mode-3` (setup scenario `routing`). Opening request: *"Let's work
through the direction for next quarter and settle the vision before we commit anyone to it."*
Date: 2026-07-03.

## 1. Pre-flight

1. **`.jit/` exists** at repo root. Pass.
2. **`jit recover`** exit 0: cleaned 1 stale lock (`.git/jit/locks/rules.lock`), rebuilt index and
   claims index; 0 leases evicted, 0 temp files. Permitted lock cleanup.
3. **Read `.jit/config.toml`**: `strategic_types = ["milestone","epic"]`; `types = {milestone=1,
   epic=2, story=3, planning=3, breakdown=3, task=4, bug=4, enhancement=4}`; `[documentation]`
   (development_root `dev`, permanent_paths `["docs/"]`, active `dev/active`).
4. **Read `.jit/templates.toml`**: one `[[template]]` (`plan`), `applies_to = ["epic"]`; union
   `{epic}`.
5. **Read content standards** at `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`
   (readable). Pass.
6. **Resume read-back**: not triggered — the request names no strategic container, so no
   vision/progress artifact to resolve or read back.

## 2. Tier derivation

Primary path (both config inputs present).

- Inputs: STRATEGIC LIST = `["milestone","epic"]` (`config.toml:31`); BOUNDARY SET = union of
  `applies_to` = `{epic}` (`templates.toml:12`); LEVEL MAP `{milestone:1, epic:2, …}` (`config.toml:28`).
- **ANCHOR TYPE = `milestone`** — first entry of STRATEGIC LIST.
- **BOUNDARY SET = `{epic}`**.
- **SHAPE = two tier** — two entries and anchor `milestone` ∉ BOUNDARY SET.
- Assumption A: `milestone(1) ≤ epic(2)` holds. Assumption B: `epic(2) ≥ milestone(1)` holds.

## 3. Mode routing

**Routed to MODE 3** (steering discussion).

Triggering signal: "Let's work through the direction for next quarter and settle the vision before
we commit anyone to it." is deliberation, not delivery: "work through the direction … settle the
vision" is interactive vision/decision work, and "before we commit anyone to it" explicitly commits
no execution and dispatches no workers — Mode 3's defining signal (*deliberate, not dispatch*).

Did NOT match the other three:
- **Not Mode 1** — no existing strategic container named or pointed at (no id, no name/label
  resolving to a `milestone`-tier container).
- **Not Mode 2** (the key disambiguation) — both modes hand to `jit-planning-lead`; the split is
  delivery intent. Mode 2 is a cold-start goal to plan **and get built**. Here "before we commit
  anyone to it" is an explicit refusal to commit execution: deliberation without delivery is Mode 3.
- **Not Mode 4** — no project-wide content-standards audit or mechanical-fix request.

Exactly one mode signalled → no stop-and-ask.

## 4. Handoff begun (Mode 3)

- **Handoff target**: the `jit-planning-lead` skill invoked **at the strategic altitude** —
  specifically its interactive strategic-altitude planning (vision/decision deliberation), with the
  container deliberated at the anchor tier (`milestone`), not below it.
- **No worker/execution lead dispatched** from the front door. Execution, if it follows, is a later
  Mode 1 invocation over a then-existing container.

**HALTED at the handoff boundary.** Did not read into or follow `jit-planning-lead`'s body, run a
planning interview, or reimplement/run any planning.

## 5. Command / state log

Under `/tmp/jit-plead-evals/routing-mode-3`: `jit recover` (permitted lock cleanup). No other jit
commands were needed — tier derivation and mode routing were satisfied from the config/templates
files directly (the fallback `jit config show-hierarchy --json` was not needed, both primary inputs
present). Plus read-only reads of the skill, its references, config, and templates.

**No `.jit/` issue state created or mutated.** No issue created, updated, transitioned, labelled, or
dep-wired; no doc link added; no vision/progress artifact written. Only side effect: `jit recover`'s
lock/index cleanup (permitted).
