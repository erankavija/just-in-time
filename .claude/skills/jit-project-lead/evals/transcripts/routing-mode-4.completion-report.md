# routing-mode-4 — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/routing-mode-4` (setup scenario `routing`). Opening request: *"Sweep the whole
project for content-standards violations, auto-fix the mechanical ones, and list what needs a
decision."* Date: 2026-07-03.

## 1. Pre-flight

1. **`.jit/` exists** at repo root. Pass.
2. **`jit recover`** clean: 1 stale lock cleaned (`.git/jit/locks/rules.lock`), index rebuilt, claims
   index rebuilt from log, 0 leases evicted. Permitted lock cleanup.
3. **Read `.jit/config.toml`**: `types = {milestone=1, epic=2, story=3, planning=3, breakdown=3,
   task=4, bug=4, enhancement=4}`; `strategic_types = ["milestone","epic"]`; `[documentation]`
   (development_root `dev`, permanent_paths `["docs/"]`, active `dev/active`).
4. **Read `.jit/templates.toml`**: one `[[template]]` (`plan`), `applies_to = ["epic"]`.
5. **Read content standards** at `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`
   (present and readable, 6994 bytes). Pass.
6. **Resume read-back**: not triggered — the mode-4 project-wide sweep request identifies no
   strategic container, so the "as soon as the request identifies the strategic container"
   precondition is not met. No vision/progress artifacts resolved.

## 2. Tier derivation

Primary path (both config inputs present, non-empty).

- Inputs: STRATEGIC LIST = `["milestone","epic"]` (`config.toml:31`); BOUNDARY SET = union of
  `applies_to` = `{epic}` (`templates.toml:12`); LEVEL MAP `{milestone:1, epic:2, …}` (`config.toml:28`).
- **ANCHOR TYPE = `milestone`** — first entry of STRATEGIC LIST.
- **BOUNDARY SET = `{epic}`**.
- **SHAPE = two tier** — two entries and anchor `milestone` ∉ BOUNDARY SET.
- Assumption A: `milestone(1) ≤ epic(2)` holds. Assumption B: `epic(2) ≥ milestone(1)` holds.

## 3. Mode routing

**Routed to MODE 4** (standards sweep).

Triggering signal: "Sweep the whole project for content-standards violations, auto-fix the
mechanical ones, and list what needs a decision" is a *project-wide content-standards audit*
(auto-fix mechanical + surface judgment calls) — Mode 4's defining signal.

Did NOT match the other three:
- **Not Mode 1** — the request names/points at no strategic container (no id, name, or label
  resolving to an anchor-tier `milestone` container).
- **Not Mode 2** — no high-level goal to scope-and-deliver; an audit is not a cold-start objective
  needing a new container.
- **Not Mode 3** — no interactive vision/decision deliberation; the request asks for delivery of
  fixes plus a findings list, not "think through direction with no dispatch".

Exactly one mode signalled → no stop-and-ask.

## 4. Handoff begun (Mode 4)

- **Section entered**: the skill's `## Standards sweep mode` (Mode 4 body), which is defined to read
  `references/standards-sweep.md` in full and then run `scripts/standards-scan.sh` →
  `scripts/standards-fix.sh` → single sweep report.

**HALTED at the handoff boundary.** Did NOT run the mode body: `references/standards-sweep.md` was
not opened for execution, `scripts/standards-scan.sh` was NOT run, `scripts/standards-fix.sh` was NOT
run, and no sweep report (Auto-fixed / Needs-a-decision) was produced. No execution lead dispatched,
no planning interview, no wave dispatch.

## 5. Command / state log

Under `/tmp/jit-plead-evals/routing-mode-4`: `jit --version` (read-only); `jit recover` (permitted
lock cleanup, touches locks/index not issue content). Plus filesystem reads: `ls` on repo/.jit and
the content-standards doc; reads of `config.toml`, `templates.toml`, the skill, and its two
references.

**No `.jit/` issue state created or mutated.** No `jit issue`, `jit doc`, `jit gate`, `jit apply`,
`jit dependency`, or any write command was run. Only side effect: `jit recover`'s stale-lock/index
cleanup (permitted).
