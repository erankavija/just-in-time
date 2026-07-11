# Tutorials audit notes (4c33d0e5)

Footprint: `docs/tutorials/` — `README.md`, `quickstart.md`, `first-workflow.md`,
`parallel-work-worktrees.md`. Binary verified at HEAD (`jit 0.2.1`, commit `8e4acd98`).

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
- Agent identity: `JIT_AGENT_ID` env var and `~/.config/jit/agent.toml` `[agent] id/description`
  (`agent_config.rs:6,19,31-37`, `errors.rs:1029`).
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
