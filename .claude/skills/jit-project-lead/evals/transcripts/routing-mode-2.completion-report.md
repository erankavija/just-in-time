# routing-mode-2 — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/routing-mode-2` (setup scenario `routing`). Opening request: *"Stand up
multi-tenant support for the product — figure out what that means and get it delivered."*
Date: 2026-07-03.

## 1. Pre-flight

1. **`.jit/` exists** at repo root. Pass.
2. **`jit recover`** exit 0: cleaned 1 stale lock (`.git/jit/locks/rules.lock`), rebuilt index and
   claims index. Permitted lock cleanup.
3. **Read `.jit/config.toml`**: `strategic_types = ["milestone","epic"]`; `types` map;
   `[documentation]` (development_root `dev`, permanent_paths `["docs/"]`, active `dev/active`).
4. **Read `.jit/templates.toml`**: one `[[template]]` (`plan`), `applies_to = ["epic"]`; union
   `{epic}`.
5. **Read content standards** at `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`
   (readable). Pass.
6. **Resume read-back**: not triggered — a cold-start request identifying no strategic container, so
   nothing to resume. First-invocation branch.

## 2. Tier derivation

Primary path (both config inputs present).

- Inputs: STRATEGIC LIST = `["milestone","epic"]` (`config.toml:31`); BOUNDARY SET = union of
  `applies_to` = `{epic}` (`templates.toml:12`); LEVEL MAP `{milestone:1, epic:2, …}` (`config.toml:28`).
- **ANCHOR TYPE = `milestone`** — first entry of STRATEGIC LIST.
- **BOUNDARY SET = `{epic}`**.
- **SHAPE = two tier** — two entries and anchor `milestone` ∉ BOUNDARY SET.
- Assumption A: `milestone(1) ≤ epic(2)` holds. Assumption B: `epic(2) ≥ milestone(1)` holds.

## 3. Mode routing

**Routed to MODE 2** (plan and execute a vague high-level goal, cold start).

Triggering signal: "Stand up multi-tenant support for the product — figure out what that means and
get it delivered." states a high-level objective with **no resolvable container**, asking both to
scope it ("figure out what that means") and deliver it ("get it delivered") — Mode 2's defining
signal, a goal without a resolvable container asking to plan and get it built.

Did NOT match the other three:
- **Not Mode 1** — no id/label; no existing anchor-tier (`milestone`) container resolves. The only
  seeded milestone ("Demo release readiness", `5fa94d63`) and its epics ("Query surface" `057aeacb`,
  "Ingest pipeline" `0b4d0cf3`) do not relate to "multi-tenant support".
- **Not Mode 3** — the request commits to delivery ("get it delivered"), not deliberation only.
- **Not Mode 4** — nothing about a content-standards audit/sweep.

Exactly one mode signalled → no stop-and-ask.

## 4. Handoff begun (Mode 2)

- **Handoff target**: the `jit-planning-lead` skill invoked **at the strategic altitude** (the
  `milestone` anchor tier), entering at its cold-start / research-and-plan (interview) entry. The
  steward supplies the vague goal and the anchor tier; planning-lead owns the interview and the plan.
  The strategic-tier container does not exist yet and is created by planning-lead at the anchor tier.
- **No worker/execution lead dispatched** from the front door. Execution, if it follows, is a later
  Mode 1 invocation over the then-existing container.

**HALTED at the handoff boundary.** Did not enter or reimplement the planning interview, run
research-and-plan, create a container, or dispatch any execution lead.

## 5. Command / state log

Under `/tmp/jit-plead-evals/routing-mode-2`: `jit --version`; `jit recover` (permitted lock
cleanup); `jit issue list --json` ×2 (read-only, to confirm no existing container resolves to the
request). Plus read-only reads of `SKILL.md`, `references/tier-derivation.md`,
`references/mode-routing.md`, `config.toml`, `templates.toml`, and the content-standards doc.

**No `.jit/` issue state created or mutated.** No issue created, edited, transitioned, labelled,
linked, or gated; no charter/progress artifact written. Only write: `jit recover`'s stale-lock
cleanup (permitted).
