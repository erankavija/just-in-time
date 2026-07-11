# Tutorials audit notes (4c33d0e5)

Footprint: `docs/tutorials/` — `README.md`, `quickstart.md`, `first-workflow.md`,
`parallel-work-worktrees.md`. Binary verified at HEAD (`jit 0.2.1`, commit `8e4acd98`).

## Rework attempt 1 — 7 scoped `doc-review` findings (all fixed, verified vs HEAD)

The first pass under-audited `parallel-work-worktrees.md` (I had wrongly listed several of
its claims as "verified accurate"). Corrected below; each fix swept footprint-wide.

- **F1 [medium] quickstart — only the 4-state happy path shown.** Kept the happy path but
  added a pointer to the full state machine (`gated`/`rejected`/`archived`) at
  [`../concepts/core-model.md#states`], at both the AI-orientation bullet and the
  "Key Concepts Learned" recap. Before: "states: backlog → ready → in_progress → done".
- **F2 [high] first-workflow — real checker execution mislabeled "simulating".** The auto
  gates were defined `--checker-command "cargo test"` / `"cargo test --test integration"`,
  which actually run and fail in the tutorial's non-Cargo project. Changed both to
  self-contained always-pass stand-ins (`echo '… passed'`) with a comment to point
  `--checker-command` at a real suite (pytest/cargo test) in a real project. Removed the
  "(simulating CI and human review)" framing; auto gates now evaluate with no `--by` (the
  checker runs), the manual `review` gate keeps `--by` attestation. Verified end-to-end.
- **F3 [high] parallel-work — agent.toml example missing required `created_at`.** Verified
  `AgentSection.created_at` is a required `String` (`agent_config.rs:35`, no serde default);
  the `[agent] id + description`-only example would fail to parse. Added
  `created_at = "2026-01-06T12:00:00Z"`. Only one agent.toml example in the footprint.
- **F4 [high] parallel-work — false auto cross-worktree visibility.** Reworded: another
  worktree sees merged issue updates only after it updates its own branch
  (`git pull`/`merge`/`rebase`); worktrees on different branches don't share commits
  automatically. Verified against git worktree behavior.
- **F5 [medium] parallel-work — Mermaid diagram showed non-UUID `task-1.json`.** Changed the
  node to `.jit/issues/{id}.json`, consistent with the `{id}.json` placeholder in
  storage-format / CLAUDE.md.
- **F6 [high] parallel-work — leases overstated as preventing conflicts.** Verified: lease
  **acquisition** is exclusive (a second `claim acquire` fails — "already claimed by …"),
  but the lease is **advisory** (CLAUDE.md "Advisory work leases"); an agent that skips
  claiming can still mutate the issue, and write-enforcement is configurable via
  `enforce_leases` (default `strict`, `config.rs:1723`). Reworded the intro, "What You'll
  Learn", Step 4 lead-in, the lease bullet, and `README.md`'s index line to frame leases as
  exclusive-hold coordination signals (not data-conflict prevention), with a pointer to
  `configuration.md#enforce_leases`. Class swept across both files.
- **F7 [high] quickstart — `jq` missing from prerequisites.** Added `jq` to quickstart
  prerequisites. Swept: `first-workflow.md` also pipes `jq`, so added it there too;
  `parallel-work-worktrees.md` uses no `jq`.

Post-rework mechanical bar over `docs/tutorials/`: M2/M3/M5 clean (exit 0; new `#states`
and `#enforce_leases` anchors resolve), M4 no box-drawing, M1 invented-flag residue now
empty (the former `--test` residue was the `cargo test --test` checker, now removed).

## Rework attempt 2 — incomplete class sweep from attempt 1

Attempt 1 fixed the CITED F4/F6 lines but missed sibling instances of the same two classes —
the "fix the line, miss the class" trap. Attempt 2 ran an EXHAUSTIVE class sweep (the two
mandated greps plus a broader propagation grep) across all four files and fixed every hit:

- **first-workflow.md:327 (F6 class — lease/claim conflict overstatement).**
  before: "**Agent Claiming**: Atomic assignment, no conflicts".
  after: "**Agent Claiming**: Atomic, exclusive assignment that coordinates who works on each
  issue". (`jit issue claim` acquisition is atomic/exclusive — a second claim on a claimed
  issue fails — but "no conflicts" overstated data-conflict prevention.)
- **parallel-work-worktrees.md:201 (F4 class — cross-worktree visibility).**
  before: "**Visibility** spans all worktrees" — false, and it contradicted line 199
  ("Issue data is per-worktree (isolated)") and the corrected line 167.
  after: the summary block now reads consistently — issue data per-worktree/branch; claims
  shared via `.git/jit/`; issue changes sync via git and are seen only after a branch update
  (with the Step-3 main-read fallback noted).
- **parallel-work-worktrees.md:208 (F4 class — same, not matched by the mandated grep but
  the same class).** before: "Complete an issue in one worktree, commit, and see it update in
  the other". after: "…commit it, then update the other worktree's branch (merge/pull) to see
  the change there".

**Line 234 verdict (`Dependencies work across worktrees:`) — reworded, accurate as a read.**
Verified empirically: from a secondary worktree, `jit graph deps <main-issue>` correctly
reads main's committed dependencies via the read fallback, but main does NOT see a secondary
worktree's uncommitted new dependency (no auto-propagation across branches). The scenario's
commands are read-only (`jit graph deps`), so the capability is real; tightened the header to
"You can query an issue's dependencies from any worktree:" to remove any auto-propagation
ambiguity.

Remaining worktree-mentioning lines are accurate, not overstatements: line 167 (corrected
F4), line 200 ("Claims are shared across all worktrees via `.git/jit/`" — verified true),
line 201 (corrected summary). Post-sweep: mandated GREP A empty; GREP B and the broad
propagation grep return only accurate statements. Mechanical bar clean (M2/M3/M5 exit 0, M4
none, M1 empty).

## Mechanical bar (post-edit, over `docs/tutorials/`)

- M2 links & anchors: `OK: all links and anchors resolve`
- M3 citations: `OK: all cited paths and @/ items resolve`
- M5 projections: `OK: projections fresh` (global; not hand-edited)
- M1 invented-flag guard: only residue is `--test` (the `cargo test --test integration`
  checker command in `first-workflow.md` — a cargo flag, expected residue, not jit surface).
- M4 diagram-shaped box-drawing art: **none** (`grep -rnP '[\x{2500}-\x{257F}]'` empty).
  The single diagram in the footprint (`parallel-work-worktrees.md` "How It All Works
  Together") is already a proper `mermaid` flowchart. REQ-04 satisfied, zero conversions.

## Drift classes found and swept

1. **Postcheck gates narrated as prechecks / manual readiness (engine semantics).**
   `first-workflow.md` treated the tasks' default (postcheck) gates as prechecks that block
   readiness and required a manual `--state ready`. Verified against source + a live HEAD
   binary: `GateStage` defaults to `Postcheck` (`crates/jit/src/cli.rs:1301`,
   `crates/jit/src/domain/types.rs:1019-1023` — postcheck runs `in_progress → gated`, blocks
   `done`, never `ready`); an issue with no dependencies is created directly in `ready`; a
   container auto-readies once its dependencies reach a terminal state. Swept 5 instances:
   Step 3 "Nothing ready (tasks have unpassed gates)" (false — tasks are ready); Step 4
   retitled/rewritten (was "Pass Prechecks and Mark Ready" — dropped the redundant
   `--state ready` transitions and the precheck framing); Step 6 dropped TASK4's pre-claim
   gate/`--state ready` block (TASK4 is ready immediately); Step 7 now passes each task's
   postcheck gates before `--state done` (moved here from the old precheck step, so the
   sequence stays mechanically valid); Step 8 dropped the epic's redundant `--state ready`
   and reordered to claim → pass gates → done. `quickstart.md`'s gate section was already
   correct (postcheck `done → gated` diversion) — no change. Full rewritten sequence
   re-run end-to-end on a HEAD binary: 3 tasks available, epic blocked, TASK4 in_progress on
   claim, epic auto-ready after tasks done, epic done, 5 done total.

2. **Over-stated requirement.** `quickstart.md:21` said labels are
   "(REQUIRED: `type:*`)". False and self-contradicting — the same doc's "Labels are
   Optional" section creates an issue with no `--label`, and a HEAD binary confirms no-label
   create succeeds (a `type:task` default is applied with an orphan warning; the type label
   is not user-required). Removed the parenthetical.

3. **Duplicated H1 + Diátaxis frontmatter block** at `quickstart.md` top (the `# Quickstart`
   + Diátaxis/Time/Goal block appeared twice). Removed the duplicate.

Minor: `quickstart.md:130` "(no longer blocked)" is illustrative of the example issue's
state, not product-legacy narration (excluded by the doc-review carve-out), but reworded to
"(TASK1 is done, so it's unblocked)" to remove any marker ambiguity.

## Claims verified accurate (no change)

- `worktree info` output shape (ID `wt:…`, Branch, Root, Type, Common dir) — matches live output.
- Claim lease: default TTL 600s / 10 min (`coordination.default_ttl_secs`, verified live);
  `claim release <issue-id>` resolves by issue, not lease UUID (`cli.rs:2587-2591`);
  `claim renew <lease-id> --extension`, `claim force-evict <lease-id> --reason` positionals correct.
- Claim storage: `.git/jit/claims.jsonl` + `.git/jit/claims.index.json`
  (`claim_coordinator.rs:206,703`) — matches the Mermaid diagram.
- Agent identity: `JIT_AGENT_ID` env var and `~/.config/jit/agent.toml` — `[agent]` section
  with **required** fields `id`, `created_at` (ISO 8601), `description` and optional
  `default_ttl_secs` (`agent_config.rs:31-41`; `created_at` has no serde default, so an
  example omitting it fails to parse — see rework F3).
- Short-hash minimum 4 chars (`storage/json.rs:643`, `storage/mod.rs:229`).
- `graph deps --depth 0` = all transitive/unlimited (`cli.rs:2004,2018`).
- 3-tier worktree visibility (local `.jit/`, git HEAD, main worktree) — confirmed
  (`storage/errors.rs:35`).
- `dev/design/worktree-parallel-work.md` inline citation resolves.

## REQ-06 (volatile facts)

The only config-default value cited in the footprint is the claim TTL (600s / 10 min). It is
already documented in `docs/reference/configuration.md:276` and the parallel-work tutorial
already cross-links "[Configuration Reference](../reference/configuration.md) — Customize
TTL…", so the fact reaches an existing citation surface. **No missing-projection-surface fact
recorded for follow-up filing from this footprint.**

## Judgment calls

- Kept the simplified happy-path lifecycle "backlog → ready → in_progress → done"
  (`quickstart.md`) — an orientation aid, not a completeness claim about the 7-value `State`
  enum; the full enum is reference-doc territory.
- Rewrote (rather than relabeled) the first-workflow gate steps so gate evaluation lands at
  completion, faithfully modelling postcheck semantics; verified the rewritten command
  sequence runs clean end-to-end.
