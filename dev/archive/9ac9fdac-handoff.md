# Handoff — Graph templates: configurable parameterized subgraphs applied to the work DAG (9ac9fdac) — session 1

**Date:** 2026-06-23
**Session number:** 1
**Prior handoffs:** None

## Current state

- Epic: `9ac9fdac` — state: **backlog** (blocked by incomplete children; promotes when all done)
- Wave in progress: **Wave 9 of 10** (impl-interior wave plan in the progress file)
- Children summary: **4 of 6 stories done**, **8 of 13 leaf tasks done**.
  - Stories DONE: `04559558` (W1 template-registry), `643874f2` (W2 apply-engine), `3143ad87` (W3 scaffold-switch), `96fc8cd8` (W4 planning-config-removal).
  - Stories REMAINING: `adc640e4` (W5 skills-docs-migration), `99a0c161` (W6 template-testing-dogfood).
  - Tasks DONE: `45e7f203`, `8b15b679`, `14137e1a`, `73e5e853`, `9cccfe0c`, `e75805f6`, `c703a4b3`, `3a7e13cf`.
  - Tasks REMAINING: `6e38a577`, `c8aa199e` (W5); `840e155c`, `57f52018`, `fc414353` (W6). The three Wave-9 tasks just auto-promoted backlog→ready.
- Active claims: **None** (all sub-agents idle; remaining tasks unclaimed).
- Open escalations: **None** (two resolved this session — see below).
- Progress file: `dev/active/9ac9fdac-progress.json`.
- Bracket gates (coverage-preview, breakdown-review) on the breakdown node `59118ee9` were already PASSED before this session.

## What just happened

- Resolved a pre-existing repo-wide blocker (commit `46374a1`): code-review requires `jit-validate` green, which failed on 16 epics missing plan docs + an isolated `WAVE-DEBUG probe` (`313e01f6`) + 10 redundant edges. User approved stub-plan-doc backfill; connected the probe (deletion was denied); `jit validate --fix`.
- W1 `45e7f203` done (`1bd86c1`): templates.rs types + loader. Rework: added `# Examples` to 6 public types.
- W1 `8b15b679` + story `04559558` done (`b4af01f`): authored `.jit/templates.toml` plan template + registry accessors. Reworks: load templates on config-absent path; git-track templates.toml; validate `applies_to` against hierarchy (`UnknownAppliesToType`).
- W2 `14137e1a` done (`d7b20c4`): apply_template core. **4 reworks**, all the validate-before-mutate class (gate-presets, labels, transform-kinds-later, then `expect()`); `prevalidate_node_writes` projects each node's final issue + runs `validate_for_write` up front.
- W2 `73e5e853` + story `643874f2` done (`706b446`): edge wiring + `move-upstream-to-role`. Story review found the **cycle-atomicity gap** → ESCALATED (4th validate-before-mutate occurrence); user chose "fix it" → `prevalidate_acyclic` simulates the full prospective edge set before mutation.
- W3 `9cccfe0c` done (`a270f95`): `jit apply` CLI (additive). First-try pass.
- W3 `e75805f6` + story `3143ad87` done (`5af6122`): atomic scaffold switch — `jit apply plan` is the sole scaffold, breakdown consumes pre-created B, deleted `plan.rs` / `jit plan` / `--with-planning`. Story review found `apply plan --force`-after-breakdown duplication (find_applied_breakdown scanned only direct deps) → fixed with store-wide `brackets:<C>` lookup; removed `expect()`.
- W4 `c703a4b3` done (`6c72f26`): migrated `validate_scope` + `resolve_plan_content` to the template registry; added `{container.id}` plan-doc alias. Rework: stale `[planning]` doc narrative.
- W4 `3a7e13cf` + story `96fc8cd8` done (`2843505`): removed `PlanningConfig` + every flat `[planning]` block; converted example configs to `templates.toml`; migrated user docs. **CFGB-02 took 4 code-review rounds** (see Traps). code-review also surfaced two PRE-EXISTING unrelated items — fixed: `#![deny(unsafe_code)]` added to all 4 crate roots (CLAUDE.md-mandated, was never declared), `# Examples` added to `ClaimCoordinator::with_fsync`.

## What to do next

- [ ] **Wave 9 — dispatch all 3 in parallel** (all `ready`, depend only on done story `96fc8cd8`; no file conflicts: skills/docs vs crates/tests vs data):
  - [ ] `6e38a577` (documentation): migrate `.claude/skills/project-lead` + `jit-breakdown` (SKILL.md, references/bracket-spine.md) + the bracket-command memory to `jit apply plan` and "B created by apply, breakdown consumes it". DOCA-01. **ALSO** rewrite `docs/concepts/planning-bracket.md`'s superseded scaffold narrative to C→B→P (see Trap 9 / `w5_carryover`).
  - [ ] `840e155c` (implementation): W6 unit/integration/property tests. TSTA-01/02 enumerate: `--force` idempotence, update-not-duplicate, JSON apply output, anchor binding, attached gates, acyclicity, transitive reduction, pre-apply snapshot transform — **much is already covered** in `crates/jit/tests/template_apply_tests.rs`; this task adds proptest invariants (per `storage/claim_coordinator_proptests.rs` pattern) + any gaps, all in isolated temp/in-memory repos.
  - [ ] `57f52018` (implementation/data): manually reseed the empty planning-node descriptions `1eb0bdfd` and `2937919f` from their containers (one-time data repair; use `jit issue update <id> --description` per the template's planning-node description shape). RESEED-01.
- [ ] Close each story when its tasks land + gates pass: `adc640e4` (after 6e38a577 + c8aa199e), `99a0c161` (after 840e155c + 57f52018 + fc414353). **Run story gates BEFORE committing the wave** (Trap 6).
- [ ] **Wave 10**: `c8aa199e` (user docs + bracket-id convention; closes story `adc640e4`), `fc414353` (dogfood `jit apply plan` on a real container; closes story `99a0c161`).
- [ ] **Epic completion** (skill Section 10): verify all children done, map REQ-01..10 → delivering issues, completion report, transition epic to done, archive + link.

## Traps — do not repeat these

1. **STALE LSP/harness diagnostics during long agent edits.** Multiple false alarms this session (`cli.rs:39 E0004`, `claim_coordinator_proptests.rs io_config dead_code`, `breakdown.rs config/find_breakdown_node errors`) were captured mid-edit and did NOT reflect the final tree. **Always confirm with `cargo build --workspace` / `cargo test` before reacting to a diagnostic.**
2. **cargo-ci and code-review must run SEQUENTIALLY, cargo-ci first.** code-review ingests cargo-ci's stored "N passed" summary as trusted evidence. After ANY code change, **refresh cargo-ci before re-running code-review**. cargo-ci stamps HEAD's commit but tests the working tree, so it validates uncommitted changes.
3. **"Remove X everywhere" criteria (CFGB-02) are enforced expansively — do ONE exhaustive sweep up front.** CFGB-02 took 4 code-review rounds because each piecemeal fix left another `[planning]` spot: example `config.toml` → `adopt-planning-bracket.md` → `gate_presets/planning.rs` doc comments + `dev/` notes → example `rules.toml` comments. For any "X gone repo-wide" criterion, run `grep -rn '<token>' crates/ docs/ dev/` first and fix every hit in one pass. `.jit/` (append-only logs) and `target/` are legitimately excluded.
4. **code-review surfaces PRE-EXISTING repo-wide debt unrelated to the task** and blocks the gate on it. Hit twice: the `jit-validate`/16-missing-plan-docs blocker, and `#![deny(unsafe_code)]`-not-declared + `with_fsync` missing-`# Examples`. Expect more on remaining waves (e.g. other public APIs missing `# Examples`). Fix small/convention-mandated ones inline; escalate genuine shared-infra changes.
5. **The apply engine's validate-before-mutate precondition phase is now COMPLETE — keep it that way.** `apply_template_with` pre-validates container type, anchors, gate presets, projected node writes (`validate_for_write`), transform kinds, AND acyclicity (`prevalidate_acyclic`) before the first `create_issue`. Took 5 rounds to get here. If you touch the engine, do not let any new mutation escape pre-validation.
6. **Run STORY gates before committing the wave.** Story code-review catches cross-task bugs no single task review sees (it found the force-after-breakdown duplication on `3143ad87`). Run them while the wave's diff is still uncommitted so the reviewer has content to review. (User explicitly chose to KEEP+run story gates even though they're aggregators.)
7. **`brackets:` label uses the container SHORT id, not full id.** The apply engine (`find_applied_breakdown`) and breakdown (`find_breakdown_node`) both match `brackets:<C-short-id>`. The pre-refactor breakdown wrote full-id; that was reconciled to short-id. Keep new code on short-id.
8. **Leave the prior-worker strays uncommitted.** `dev/sessions/session-20260622-planning-failures-and-churn.md` (untracked) and the `eed6750c` doc-add (modified `.jit/issues/eed6750c-*.json`, links that session note) are leftover from before this session — not part of this epic. Do not stage them.
9. **`docs/concepts/planning-bracket.md` still teaches the SUPERSEDED scaffold model** (B created by the breakdown step; "at scaffold only C and P exist"; "C→P disappears after breakdown"). Actual behavior is now `jit apply plan` creates B at scaffold → C→B→P. `6e38a577`/`c8aa199e` must rewrite this to the apply-creates-B / C→B→P model. (No `[planning]` block, so it did not block W4.)

## Open questions needing user input

None. (Two escalations this session were resolved: jit-validate blocker → user chose stub-plan-doc backfill; apply-engine cycle-atomicity → user chose the edge-simulation fix. Both done.)

## Reference artefacts

- Epic: `jit issue show 9ac9fdac`
- Plan/spec (the breakdown's source of truth): `dev/active/9ac9fdac-828a-40f2-9d73-71af14f44ff8-plan.md`
- Progress file: `dev/active/9ac9fdac-progress.json`
- This repo's authored template: `.jit/templates.toml`; example templates: `docs/examples/{sdd,research}/templates.toml`
- Apply engine: `crates/jit/src/commands/template.rs`; registry: `crates/jit/src/templates.rs`; breakdown (consumes B): `crates/jit/src/commands/breakdown.rs`
- Commits this session: `46374a1` (repo-health), `1bd86c1`, `b4af01f`, `d7b20c4`, `706b446`, `a270f95`, `5af6122`, `6c72f26`, `2843505`.
