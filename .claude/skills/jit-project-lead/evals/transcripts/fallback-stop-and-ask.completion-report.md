# fallback-stop-and-ask — Completion Report

Runner: harness-governed fresh-context sub-agent given the scenario `prompt` verbatim
(equivalent-runner path per `docs/reference/skill-eval-adjudication.md`). Repo:
`/tmp/jit-plead-evals/fallback` (setup scenario `fallback`: bare `jit init`, no `.jit/templates.toml`).
Date: 2026-07-03.

## 1. Pre-flight

- **Verify `.jit/` exists** — present at `/tmp/jit-plead-evals/fallback/.jit/`: `config.toml`,
  `events.jsonl`, `gates.json`, `index.json`, `issues/`, `rules.toml`, `schemas/`. **No `templates.toml`.**
- **`jit recover`** — exit 0. Cleaned 1 stale lock (`.git/jit/locks/rules.lock`), rebuilt index and claims
  index, 0 leases, 0 temp files. Housekeeping only.
- **Read `.jit/config.toml`** — `types = { milestone=1, epic=2, story=3, task=4 }`;
  `strategic_types = ["milestone", "epic"]` (present, non-empty).
- **Read `.jit/templates.toml`** — ABSENT; the `applies_to` union has no source.
- **Read canonical content standards** — readable at
  `/home/vkaskivuo/Projects/just-in-time/docs/reference/jit-content-standards.md`.

## 2. Path taken: FALLBACK

Derivation guard (procedure step 1): absent `.jit/templates.toml` (BOUNDARY SET has no source) or empty
STRATEGIC LIST → go to Fallback. Missing input: **`.jit/templates.toml` is absent**. STRATEGIC LIST was
*not* missing (`strategic_types = ["milestone", "epic"]` present). Single trigger: the missing templates file.

## 3. Recovered proposal

LEVEL MAP from `jit config show-hierarchy --json` (non-type `message` key dropped):
`{ milestone:1, epic:2, story:3, task:4 }`. Non-empty, exit 0 → hierarchy usable.

- **Anchor = `milestone`** — fallback step 2: STRATEGIC LIST present, so anchor = its first entry.
- **Boundary = `{ epic }`** — fallback step 3: BOUNDARY SET has no source, so it is the type(s) at the next
  distinct level below the anchor. Anchor `milestone` is level 1; next distinct level is 2, sole member `epic`.
- **Shape = two tier** — two strategic entries and anchor `milestone` ∉ boundary `{ epic }`.
- **Assumption A: PASS** — levels `1, 2`; `1 ≤ 2` and anchor holds the minimum.
- **Assumption B: PASS** — `epic`(2) ≥ `milestone`(1).

Both pass, so the fallback produced a concrete proposal rather than stopping bare.

## 4. Stop point

Stopped at **tier-derivation Fallback step 5 ("Report and stop for confirmation")**, routed there from
procedure step 1 by the absent `templates.toml`. A **"Stop with a proposal"** outcome. The proposal is
presented as a **confirm-before-applying** ask.

- Did **NOT** apply the recovered tiers.
- Did **NOT** proceed past derivation — no shape-driven operation, no mode dispatch.
- Confirmation required before the values are emitted/used; on confirmation they must be recorded as
  numeric-level-fallback (recovered, not config-declared).

## 5. Command / state log

Under `/tmp/jit-plead-evals/fallback` only: `ls -la .jit/` and file-existence tests (read-only);
`jit recover` (cleared 1 stale lock, rebuilt index/claims index — housekeeping); `jit config
show-hierarchy --json` (read-only); reads of config.toml and content-standards doc.

**No `.jit/` issue state created or mutated. No config, templates, gates, or rules created or modified.**
The only filesystem change was `jit recover` removing a stale lock and rebuilding the index/claims index.
No repo other than `/tmp/jit-plead-evals/fallback` was touched.
