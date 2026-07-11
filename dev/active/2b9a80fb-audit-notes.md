# Audit notes — 2b9a80fb CLI command catalog (`docs/reference/cli-commands.md`)

Source-verified against HEAD `8e4acd98`. Binary `jit 0.2.1 (commit 8e4acd98)` prefix-matches HEAD.

## Mechanical bar (self-verified, worktree root)

- **M2** links & anchors: `OK: all links and anchors resolve`.
- **M3** citations: `OK: all cited paths and @/ items resolve` (was `MISSING: .jit/claims.jsonl` before fix).
- **M4** box-drawing (`grep -rnP '[\x{2500}-\x{257F}]'`): no hits — clean. No hand-drawn box/flow diagrams. Arrow tokens present are all inline `A → B` prose (state transitions), which the scope carve-out excludes from the diagram mandate; no multi-line ASCII/`-->` spines. REQ-03/04 diagrams: nothing to convert. No `mermaid` fences needed.
- **M5** projections: `OK: projections fresh` (untouched).
- **M1** invented-flag guard residue, all adjudicated non-defects: `--add-label` (visible_alias, `cli.rs:986`); `--all-targets`/`--check`/`--lib`/`--workspace` (cargo, non-jit); `--help`/`--schema`/`--version` (globals); `--quiet` (`cli.rs:29`); `--clear-` (grep truncation of `--clear-*` gate-update flags, all real). `--subtask` no longer appears (fixed, see below).

## Drift classes swept and fixed

1. **Event tag/field (REQ-07 seed).** `cli-commands.md:3756-3757`: `--event-type state_changed` + `.new_state` → real tag `issue_state_changed`, real field `to` (serde `#[serde(tag="type", rename_all="snake_case")]` on `Event`, `types.rs:1307-2030`). Also fixed the `jq` in the same block: `jit events query --json` returns the list envelope `{count, events:[...]}`, so `[.[] | select(.new_state==...)]` → `[.events[] | select(.to=="done")]`. Only one event-tag instance in the footprint.

2. **`.jit/claims.jsonl` miscitation (M3 seed).** `cli-commands.md:937-940` (`jit init`) claimed init sets a `.gitattributes` union-merge driver for `.jit/claims.jsonl` and that this stops "concurrent worktrees' claim-log" conflicts. Claims/leases actually live under `.git/jit/` (untracked per-worktree control plane: `.git/jit/claims.jsonl`, `.git/jit/claims.index.json`, verified on disk; `claim_coordinator.rs` uses `shared_jit`=`.git/jit`). Rewrote to describe the real tracked driver (`.jit/events.jsonl`) and to state that lease/claim state lives under `.git/jit/`, not the versioned `.jit/` tree.
   - **Judgment call:** `gitattributes.rs:60` literally writes a `.jit/claims.jsonl merge=union` line into `.gitattributes`, but no such file is ever created under `.jit/` — it is a dead/vestigial driver for a path jit never writes. I dropped the `.jit/claims.jsonl` citation rather than document a driver for a non-existent tracked file (over-crediting + M3 MISSING). The vestigial gitattributes line is a **code** matter, out of this doc footprint — see Group-C follow-up below.

3. **Broken `jq` envelope expressions (same class as the seed).** Barewords missing the leading `.` and one non-existent field, all in the scripting section:
   - `3592` `jq -r 'id'` → `.id`; `3595` `jq -r 'title'` → `.title`; `3598` `jq -r 'issues[]…'` → `.issues[]…`; `3601` `jq -r 'issues[0].id'` → `.issues[0].id`; `3604` `jq -r 'summary.by_state'` → `jq '{ready,in_progress,blocked,gated,done,rejected}'` (`jit status --json` has **no** `summary` object; per-state counts are top-level fields).
   - `3753-3755` `jq -r 'count'` → `.count` (three list envelopes).
   - Verified barewords actually error (`count/0 is not defined`), and the target envelope shapes: `issue create/show --json` spread the issue at top level (`.id`/`.title`); `query all/available/blocked --json` = `{count, issues:[...]}`; `status --json` top-level keys `ready/in_progress/blocked/gated/done/rejected/open/total`.

4. **Enum-value case.** `3598` `select(.priority == "High")` → `"high"` (`Priority` is `snake_case`: `low`/`normal`/`high`/`critical`). Only capitalized-enum-in-JSON instance in the file; all `"state": …` JSON values already lowercase. Confirmed `normal` (not `medium`) is the real default value — `Priority::from_str("medium")` is rejected in tests.

5. **Invented CLI surface.**
   - `878` multi-value "Applies to" list contained `--subtask` (no such flag anywhere) and `--description` (real, but single-value `String`, not comma-delimited). Corrected the list to the actual `value_delimiter` set: `--label`, `--gate`, `--add-gate`, `--remove-label`, `--remove-gate`, `--except`, `--fields`.
   - `3498` `jit invariant list` (invented — `InvariantCommands` has only `check`/`render`) → `jit item list --kind invariant` (verified working; invariants are an item kind).

6. **Over-credited behavior.** `3541-3542` `jit config validate` claimed it flags "deprecated options" — the handler (`main.rs:5424-5521`) only load-checks repo/user/env config; no deprecated-option check exists. Rewrote to "syntax errors and invalid values (both surface when a config source fails to load)".

7. **JSON-shape confusion (three distinct issue shapes).** The catalog conflated the minimal list shape, the `IssueShowResponse`, and the on-disk/`graph export --full` node shape:
   - `5-32` "CLI JSON contracts" — the flagship `issue update --json` example showed `gates_required: [...]` + `gates_status: {…}` (map with `updated_by`/`updated_at`). Real `issue update`/`show`/`create --json` return `IssueShowResponse` (`output.rs:1096`) with a **`gates` array** of `GateView` `{key, status, last_run_at, exit_code}` (no `gates_required`/`gates_status`, no `updated_by`). Rewrote to the real `gates` array.
   - `2172-2175` — "Find issues with specific gate status" filtered `jit query all --json | jq '.issues[] | select(.gates_status.tests…)'`, claiming query-all returns "the stored issue shape, which keeps the gates_status map". Real `query all --json` `issues[]` is the **minimal** shape (`assignee,id,labels,priority,short_id,state,title`) — no gate detail at all. Rewrote to `jit graph export --format json --full | jq '.nodes[] | select(.gates_status.tests.status=="failed")'` (verified: full nodes carry `gates_status`; filter runs).
   - Verified-correct: `graph export --full` node description (`2963-2968`) IS accurate — on-disk `issues/<id>.json` does carry `gates_required` + a `gates_status` map of `{status,updated_by,updated_at}` (read a live issue file); `gate define` prose reference to `gates_status` (`1666`) is a correct concept mention.
   - `3652` List-envelope table: added the missing `query divergence → divergences` row (verified `query divergence --json` = `{count, divergences:[…]}`). All other collection keys in that table spot-verified against live `--json` and correct (`gate list→gates`, `graph deps→nodes`, `graph rdeps→dependents`, `graph roots→roots`, `query count→by_state`, `gate status-all→gates`, `gate status --all→results`, `item list→items`, `gate preset list→presets`).

8. **Legacy/migration narration (REQ-05).** `3413-3416` described the `migrate lifecycle-timestamps` target as "issues created before those fields were written at transition time" (migration/transition narration about jit's own past). Rephrased to the current-property framing "issues whose stored records are missing one or more of these fields" (matches the handler, which fills only still-absent fields). No `formerly`/`previously`/`no longer`/`will be`/`coming soon` markers found elsewhere; `obsolete – No longer relevant` at `1561` is a rejection-reason *value*, not product narration.

## Verified-correct (no change)

- Preset **contents** (`2366-2405`): `rust-tdd`/`python-tdd`/`js-tdd`/`security-audit`/`minimal` + planning trio names, gate keys, prechecks/postchecks and timeouts all match `jit gate preset show`/`list` at HEAD. Custom-preset storage `.jit/config/gate-presets/<name>.json` matches `gate_presets/manager.rs:48`. Preset subcommands `apply/create/list/show` all real.
- Command aliases used in examples all real: `query ready`→available, `gate eval`→evaluate, `graph downstream`→rdeps, `graph dependencies`→deps, `dependency`/`document` family aliases, `config show`.
- `State` enum (7 variants) and the exit-code taxonomy table (`3769-3778`) match `types.rs:28-42` and `--schema.exit_codes`. `is_terminal = Done|Rejected` (so "archived, which is not terminal" at `1488` is correct).

## REQ-06 — builtin gate-preset facts (RESOLVED in doc-review rework; supersedes the earlier "acceptable as-is" judgment)

Presets live in code (`crates/jit/src/gate_presets/builtin.rs`) with no markdown projector. My first pass judged the hand-enumeration acceptable because it was currently accurate and cited the live surface; **doc-review F3 correctly rejected that** — `@/inv/single-source-prose` makes a hand-maintained copy a staleness defect *even when currently accurate*. All hand-copied builtin-preset counts/contents have now been de-hardcoded and re-pointed at the live source of truth (`jit gate preset list`/`show`); see the doc-review-rework section below.

## RESOLVED — MCP Tools Reference collapsed (lead-approved)

The former **MCP Tools Reference** (`192-865`) hand-copied a whole MCP tool catalog (`### Core MCP Tools` at `243-633`, plus JS usage examples, response-format, efficiency-tips, and testing sub-sections) that had drifted badly from the auto-generated surface:

- **Fully invented (no command even in all-tools mode):** `jit_graph_show`, `jit_query_state`, `jit_query_label`, `jit_query_priority`, `jit_query_assignee` — REQ-01 blockers.
- **Alias-named instead of canonical:** `jit_query_ready` (→ `jit_query_available`), `jit_graph_downstream` (→ `jit_graph_rdeps`).
- **Wrong hyphen/underscore convention:** `jit_gate_status_all` (→ `jit_gate_status-all`), `jit_issue_claim_next` (→ `jit_issue_claim-next`).
- **Real-but-curation-excluded, presented as "Core":** `jit_gate_show/remove/define`, `jit_graph_export/roots`, `jit_issue_list/search`, `jit_query_closed/strategic`, `jit_version`.
- Plus mangled JS (`createdid`, `readycount`, `claimedid`, `rid`), an unverified `{success,data}` transport claim, and a non-existent `node test-tool.js` command.

**Rationale (lead-approved).** The whole catalog is a REQ-06 / `@/inv/single-source-prose` violation — a hand duplicate of an auto-generated surface — so hand-correcting the shapes would only mint a fresh copy that re-drifts. Per the lead's decision, the enumerated catalog and its example/response/testing sub-sections were **collapsed to a short cited pointer**: the `## MCP Tools Reference` heading (anchor preserved) now states the generation model only — tools generated from `jit --schema`, named `jit_<command_path>` (`jit doc assets list` → `jit_doc_assets_list`), flags→params with hyphens→underscores, repeatable flags→arrays, responses = the command's `--json` payload; curated default set in [`mcp-server/curated-tools.json`](../../mcp-server/curated-tools.json); full set via `JIT_MCP_ALL_TOOLS=1` — and cites [`mcp-server/README.md`](../../mcp-server/README.md) (verified authoritative at `README.md:98-121`) as the home for install, client config, and the live tool list. Net −639 lines (3838→3199).

**Inbound links.** The `#mcp-tools-reference` heading (and thus anchor) was kept, so the three tree-wide inbound links resolve unchanged: `dev/index.md:114`, `docs/tutorials/quickstart.md:45`, `mcp-server/README.md:366`. No inbound link targets any removed sub-anchor (`#core-mcp-tools` etc. had zero inbound references). No doc was deleted/merged, so no cross-file repoint was required.
   - Minor follow-up (out of footprint, not fixed): `docs/tutorials/quickstart.md:45` describes the link as "complete tool catalog" — the catalog now lives in `mcp-server/README.md`; that inbound link still resolves but its wording could point there directly.

**Pre-existing tree-wide dangling links (NOT mine, out of scope):** a whole-tree M2 run surfaces dangling targets only in `dev/` (contributor notes/archives and `dev/authoring-conventions.md`, which contains *intentional* broken-link examples) and `web/TESTING.md`. None reference `cli-commands.md` or the collapse; all predate this change and lie outside the single-file footprint.

## Doc-review rework (round 1 — 3 findings, all fixed)

- **F1 [high] repo-local `--type bug` unframed.** `jit issue create "Fix login bug" --type bug --priority high` (create example) used `bug`, a repo-local dogfood type; `jit init` ships only `milestone/epic/story/task`. Fixed to `--type task` (shipped default). File-wide sweep: `bug`/`enhancement`/`planning`/`breakdown` as an issue TYPE appears nowhere else; no `brackets:`/`satisfies:`/`per:` repo-local label namespaces anywhere. Remaining `planning`/`breakdown` tokens are the shipped planning-bracket presets' P/B node roles (concept-doc–framed, not repo-local types); `grep "bug"` lines are title searches.
- **F2 [high] `jit migrate lifecycle-timestamps` read as historical migration.** Command kept documented; reframed to present-tense current behavior — dropped "One-time", "a second run over an already-migrated repository", and "predating event coverage". Kept every accurate fact: idempotent (writes nothing / appends no event / `issues_updated: 0` when nothing is missing), derives from `.jit/events.jsonl`, fills only absent fields, emits `lifecycle_timestamps_backfilled`, `--json` shape. Also reworded an innocent "commands used to …" in the collapsed MCP section to avoid the reviewer's `used to` grep. No other historical/migration narration remains file-wide.
- **F3 [high] hand-copied builtin-preset counts/enumerations (`@/inv/single-source-prose`).** Three instances, all de-hardcoded and re-pointed at the live source of truth (`jit gate preset list` / `show`) rather than re-copying corrected numbers:
  1. `preset list` sample output — `(5 gates)`/`(1 gate)`/`(3 gates)` → `(<N> gates)` placeholders + an annotation that the builtin registry is authoritative.
  2. `preset show rust-tdd` sample output — annotated as an illustrative layout; the concrete per-gate `Command:`/`Timeout:` values genericized to `<command>`/`<N>s` so it no longer asserts rust-tdd's authoritative contents.
  3. `### Builtin Presets` prose — replaced the per-preset hand-enumerated gate lists/timeouts/stages with names grouped by purpose (language TDD starters, `security-audit`, `minimal`, planning-bracket trio) plus the directive to introspect contents via `jit gate preset show <name>`. Preset behavior (planning-bracket attach-on-bracketing, custom-preset override) preserved.

Post-rework mechanical bar: M2/M3/M5 clean, M4 no box-drawing, M1 residue only adjudicated non-defects (cargo/global flags, `--add-label` alias, `--clear-` grep fragment).
