# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 8

**Date:** 2026-08-03
**Session number:** 8
**Prior handoffs:** `handoff.md` (1), `handoff-2.md` (2), `handoff-3.md` (3), `handoff-4.md` (5),
`handoff-5.md` (6), `handoff-6.md` (7), same directory. Every trap in every prior handoff
remains in force unless a trap here records its resolution.

## Current state

- Epic `e204e63d` — state: backlog. **78 issues** carry the epic label. **61 done** (51 at
  session start), 13 backlog, 2 in_progress, 2 ready. `jit validate` clean.
- **Wave 3 closed and wave 4 is most of the way.** Closed this session: `ebbb254f`,
  `590ff4db`, `2ce8ce1b`, `9de22aa7`, `6013cd81`, `d54f8f83`, `959274b6`, `25a33a83`,
  `59f12ba7`, and the `1c0eb82e` story checkpoint.
- **`daddfc0b` is the only worker still running** (Opus, on
  `.agents/worktrees/agent-daddfc0b`, three commits in, running its full suite).
- **`93fef456` is `in_progress` with three failed gate records** — a lead error, not a defect
  in its delivered work. See the trap on gating a container whose criterion depends on a
  non-predecessor.
- **`ff1bbada` is `ready`** and is the next dispatch, after `daddfc0b` merges.
- Progress file: `progress.json`, same directory.

## What this session did

- **Reclaimed 11 merged worktrees, freeing ~38 GB** (/ went 88% → 84%). Do this at every
  session start.
- **Closed ten issues**, every one reviewed against its criteria and gated.
- **Delivered the release-archive distribution end to end.** The native archive now carries
  both package directories under `packages/`, the archive verification asserts both, and the
  release smoke applies `jit-dogfood` from the extracted prefix and asserts each applied
  package's origin names its own extracted directory — so a dependency resolved from the
  compiled-in copy fails the assertion.
- **Caught three ordering problems in the reviewed graph**, two before they cost a wave and
  one at a gate (below).
- **Converged three duplicate `.jit/`-prefix helpers** into `VirtualPath::repository_relative()`
  and four unsynchronized `set_current_dir` test sites onto one RAII guard.
- **Closed the packaged-`satisfies` drift** (`surfaced_pitfalls`): the workflow package
  carried a pre-amendment description that a re-apply would have written over the owner's
  session-5 ruling.

## Owner rulings and lead decisions this session

Full text in `progress.json` under `owner_rulings`. In brief:

1. **`d94f6849` resequenced behind `ff1bbada`** (owner ruling). Deleting the checked-in live
   copies while the binary still embeds `profiles/jit-dogfood` makes
   `ProfilePackage::from_embedded_dir` fail with `MissingContent`, breaking the planning gate
   presets, the profile commands, profiled initialization and the template-region render.
   Edges `d94f6849 → 959274b6` and `d94f6849 → 9de22aa7` dropped; edge `ff1bbada → d94f6849`
   added. No criteria changed.
2. **`6013cd81` pulled forward beside `9de22aa7`** (lead decision, no criteria change).
   `9de22aa7`'s `code-review` failed on the archive carrying only the workflow package while
   that package declares `jit-default` as a dependency. Closing that finding is `6013cd81`'s
   entire content, so the edge `9de22aa7 → 6013cd81` was dropped and it was dispatched
   immediately. Both then passed with zero findings.

## What to do next

- [ ] **Reclaim worktrees first.** Merged and reclaimable now: `agent-2ce8ce1b`,
      `agent-590ff4db`, `agent-d54f8f83`, `agent-9de22aa7`, `agent-6013cd81`, `agent-959274b6`.
      Keep `agent-a122b9b3` and `steward-v1-readiness` (unmerged).
- [ ] **Close out `959274b6`.** Review the fix on `worktree-agent-959274b6-fix`, merge it, and
      re-run `jit gate evaluate 959274b6 code-review`. `cargo-ci` already passed.
- [ ] **Land `daddfc0b`**, the last of wave 4 still running. `59f12ba7`, `25a33a83` and
      `6013cd81` are closed.
- [ ] **Re-gate `93fef456` only after `daddfc0b` merges.** It is `in_progress` with three
      failed gate records; see the trap below. Nothing is wrong with the story's delivered
      work — its REQ-01 simply is not true until `daddfc0b` lands.
- [ ] **Dispatch `ff1bbada` after `daddfc0b` merges, not beside it.** Both rewrite
      `crates/jit/src/commands/init.rs`, `crates/jit/src/gate_presets/` and
      `crates/jit/src/profile/dogfood.rs`; the plan calls the embed-and-preset removal
      indivisible across ten files. `6013cd81` already landed, so the archive constraint that
      `ff1bbada` had to follow is satisfied.
- [ ] **Order `6013cd81` before `ff1bbada`** — already satisfied, `6013cd81` is done. Nothing
      to do; recorded so the constraint is not re-derived.
- [ ] **`d94f6849` now runs after `ff1bbada`**, and `26f503cc` after it. Do not re-order.
- [ ] Still unfiled after four sessions, both worth filing as dogfooding friction:
      `jit issue create` has no `--description-file` (unlike `jit issue update`), and
      `jit gate` cannot evaluate one gate across several issues in one command.

## Traps — do not repeat these

All prior traps remain in force; read every earlier trap section. New this session:

- **A leaf that deletes a source the binary still embeds is an ordering defect, not a task.**
  `d94f6849` was `ready` and looked dispatchable. `dogfood.rs:11` embeds
  `profiles/jit-dogfood` via `include_dir!`, and `package.rs:645` raises `MissingContent` for
  a declared source with no file, so the deletion breaks `jit_dogfood_package()` and every
  production caller of it. **Before dispatching any issue that deletes files, ask what still
  reads them at compile time.** `git grep include_dir` answers it in one command.
- **A container gate can fail on a property the plan deliberately split across two issues.**
  `9de22aa7`'s reviewer cited `@/charter/D-8` and D-16 for a two-package archive, which
  `9de22aa7`'s own Notes explicitly deferred to `6013cd81`. The No-argue discipline forbids
  arguing the reading, and absorbing the sibling's criteria empties a reviewed issue. The
  resolution that cost nothing was **landing the sibling first** — drop the edge, dispatch it,
  re-run the review. Reach for that before escalating a reviewer finding as a false positive.
- **The lead's own finding can be wrong; check the shared call path before writing the
  verdict.** `d54f8f83`'s round-1 verdict claimed `prepare_profile` never captured the
  applied-profile records, because its `content_paths` pushes only the directory. The
  discovery is in `capture_proposed_base_inner` (`commands/validate.rs:253-335`), which every
  caller reaches. The rework was still worth dispatching — the secondary check in the same
  verdict found a real defect, the registry path built by string-stripping `.jit/` instead of
  `classify_repository_relative` — but the headline finding was not.
- **`docs-mechanical` is a whole-surface checker, so one issue's stale projection fails
  another issue's gate.** `2ce8ce1b`'s `docs-mechanical` failed on drift in
  `docs/reference/rules-and-gates.md` that `590ff4db`'s rule move had caused. Do not read a
  whole-surface gate failure as the gated issue's defect until you have read what it names.
- **Removing rules from the engine leaves this repository's own derived state stale, and
  `jit validate` is where it surfaces.** After `590ff4db` merged, `.jit/rules.toml` still
  declared two rules nothing emits and `jit validate` failed with derived-state drift.
  `jit validate --fix` repaired it and `jit project render` refreshed the projection; both
  are lead-owned and neither is a worker's job. Expect this after any change to
  `default_ruleset`.
- **A test helper that shadows a production symbol hides the substitution it exists to make.**
  `959274b6`'s test module defined `jit_default_package()` — the exact name of the production
  function returning the compiled-in package — delegating to the assembled one. Renamed to
  `assembled_default_package`. Watch for this shape in every issue that moves callers from a
  compiled-in source to an assembled one.
- **A test that changes the process working directory races every other test that does.**
  `959274b6`'s CWD-independence test called `set_current_dir` with no lock and a bare
  `let _ =` restore; `cargo-ci` passed by scheduling luck and `code-review` caught it. Three
  other sites in this crate do the same (`commands/claim.rs`, `commands/hooks.rs`,
  `storage/json.rs`). A guard is being added; adopt it rather than adding a fourth spelling.
- **A worktree branch cut before a documentation relocation will conflict with it.**
  `d54f8f83`'s merge conflicted in `docs/reference/profiles.md` because `2ce8ce1b` had moved
  the enumeration fact to the command reference while `d54f8f83`'s branch, cut earlier, added
  a new paragraph beside the old copy. The resolution keeps both sides — the new paragraph
  and the citation, not the restated fact. **When one wave issue relocates a documentation
  fact, expect every branch cut before it to conflict there.**
- **Opus workers dispatched through the Agent tool go idle without reporting, twice.**
  `959274b6` was prodded twice via `SendMessage` and answered neither time, though its work
  was complete and correct. Read the branch; treat the report as a bonus. This confirms the
  session-6 trap rather than superseding it.
- **A container's criterion can depend on an issue that is not one of its predecessors, and
  graph readiness is then not gate readiness.** `93fef456`'s 13 declared predecessors were all
  done, so it looked gateable; its REQ-01 ("a repository declaring no namespaces and no type
  hierarchy receives neither") is not satisfiable until `daddfc0b` lands, because plain
  `jit init` still selects `HierarchyTemplate::default()` (`crates/jit/src/main.rs:2165`) over
  a template hardcoding the four types and seven namespaces
  (`crates/jit/src/hierarchy_templates.rs:351`). `code-review` said exactly that and was
  right; `doc-review`'s five findings are the same dependency seen from the documentation
  side. **Before gating a container, read its criteria against the tree, not its predecessor
  list against the graph** — one `jit init` in a temp directory would have shown it in
  seconds. Re-gate `93fef456` after `daddfc0b` merges.
- **A new gate line in the pipeline log means the previous gate finished, not that it
  passed.** The lead read `=== gate 93fef456 jit-validate` as evidence `cargo-ci` had passed
  and said so; three of that issue's five gates had in fact failed. Read `jit gate status-all`,
  never the progress log, for a verdict.
- **`cargo-ci`'s incremental-state check fails when a concurrent worker builds during the
  gate run.** `merge-and-gate.sh` clears `target/*/incremental` immediately before every
  evaluation, which defeats a writer that ran *before* the gate but not one that runs *during*
  it. `93fef456`'s run passed fmt, clippy, 4360 tests, provenance and the budget, and failed
  only on a `target/debug/incremental` directory that appeared mid-run while the `daddfc0b`
  worker was executing its full suite. Do not gate on main while a worker is running a full
  Rust suite.
- **An assertion can become tautological without any gate noticing, and the tell is that both
  sides now perform the same read.** After `959274b6`,
  `test_live_assets_match_every_declared_source_tree_consumer` (`dogfood.rs:955`) compares
  `package.source_bytes(&asset.source)` against `fs::read(root.join(&asset.target))` — which,
  for an assembled package, *is* that same read. `cargo-ci` and `code-review` both passed
  over it; the worker found it by reading its own diff and correctly declined to delete it.
  **Do not delete it now:** `d94f6849` REQ-02 is precisely "The assertion comparing a packaged
  copy against its repository counterpart is gone, along with the copies it compared", so
  removing it early would pre-empt that criterion's subject. Recorded in `surfaced_pitfalls`.
  A second instance of the same shape was found in the opposite direction: the compatibility
  test at `gate_presets/builtin.rs` was tautological *before* `959274b6` (both sides of the
  `assert_eq!` called `jit_dogfood_planning_gate_keys`) and is now a real cross-route
  comparison. **When an issue changes where a value is read from, re-ask what each side of
  every assertion over it now reads.**
- **A grep-derived call-site inventory in a lead brief is a starting point, not a contract.**
  The `959274b6` brief's pattern covered `jit_dogfood_gate` but not
  `jit_dogfood_planning_gate_keys`, so `gate_presets/builtin.rs` was missing from it; the
  worker re-derived the set and found it. Tell workers to re-derive rather than trust the
  list — that instruction is what caught this.
- **`codex exec` rejects an absolute path outside its `-C` directory even when that path is
  inside the worktree it was given.** The `9de22aa7` worker hit
  `patch rejected: writing outside of the project` and recovered by using the repository's own
  `apply_patch` executable. Harmless, but it costs the worker a detour; nothing to change.

## Reference artefacts

- Epic: `jit issue show e204e63d` — nine live criteria (REQ-06 retired).
- Plan: `e204e63d-plan.md`; manifest: `e204e63d-breakdown.json` — authoritative, but note
  `32779829` (session 6), `d35cc3f2` (session 7) were added on owner rulings and are not in
  it, and that the graph was re-ordered twice this session (see Owner rulings above).
- Boundary audit: `dev/active/7cbefe7c/findings.md` — **S4's reachability claim is disproven.**
- Session tooling (scratchpad, NOT committed — copy forward from the newest session
  directory): `merge-and-gate.sh` (sorts `code-review` last; supports `--gates-only`),
  `dispatch-codex.sh`, `dispatch-wave.sh`, `mkbrief.py` (takes a worktree suffix and an
  addendum file), `ws-runs.sh`, `briefs/`, `logs/`.
