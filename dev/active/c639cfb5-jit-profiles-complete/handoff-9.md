# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 9

**Date:** 2026-08-10T16:05:00+03:00
**Session number:** 9
**Prior handoffs:** `handoff.md` through `handoff-8.md` in this directory

## Current state

- Epic: `c639cfb5` — state: backlog — **now `milestone:v1.0`, not v1.1**
- Wave in progress: wave 10 of 18 (wave 9 closed)
- Children summary: 11 done, 0 in progress, 16 backlog/ready, 0 rejected (28 members after `9a81ab12` was added)
- Active claims: none
- Open escalations: none — both were answered this session (see *What just happened*)
- Main integration head: `19e3117b`; last implementation merge `cda04fb8`
- Progress file: `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Next ready: `6a479c58` (capture) and `9a81ab12` (remove shipped-v1 boundary)

## What just happened

- **The epic moved to v1.0 by owner decision.** Charter `D-8` reversed and `D-14` amended in `dev/vision/9db27a3a-charter.md`, re-projected into `AGENTS.md`/`CLAUDE.md`. All 27 then-existing epic issues relabelled `milestone:v1.0`. `bb03df0a` ("Cut the v1.0.0 tag") now depends on `c639cfb5` and moved **ready → backlog**; the `0735879a → c639cfb5` edge was removed. Commit `201c4f34`.
- **Owner settled the v1.0-record migration question as a clean cut**: "gf2 is the only consumer. No legacy." Because v1.0 has not shipped, there is no released record format to stay compatible with. Epic criteria amended accordingly — REQ-05, REQ-09, REQ-11, REQ-12, D-01, D-05, RISK-02, RISK-04 and the background framing no longer promise a v1.0-record migration.
- **Created `9a81ab12`** "Remove the shipped-v1 record migration boundary" (gates: cargo-ci, code-review; 5 criteria). 126 references across 7 files plus a 40 KB pinned evidence fixture. Removing it also dissolves `7793b8b7`.
- **Created `7793b8b7`** — `test_profiled_init_authenticates_shipped_v1_once_before_any_publication` asserts on a process-global counter; green unfiltered/isolated/single-threaded, red under `cargo test -p jit --lib profile`.
- **Created `c3a09e0d`** (this repo) and `d658b160` (gf2) — the cargo-ci build-lock defect; **both fixed and committed** (`4ce92063` here, `5c244cad` in gf2).
- **Wave 9 `7156f64b` done** after **five** code-review rounds. Final surface: `jit profile reconfigure|upgrade --profile <SELECTOR>... [--values-file] [--set]... [--dry-run] [--json]`. Merges `f6c422de`, `d5f25800`, `cda04fb8`.
- Wave 9 net structural effect was **subtractive**: three variable-resolution entry points collapsed into one parameterised by `RecordedValueAuthority`; `resolve_variables` stopped duplicating the precedence ladder; two selection paths merged; three copies of the disposition→action mapping became one; a `SelectionObservation` trait now states "publishes nothing" once per answer type.
- Tooling switched from `codex exec` to native subagents mid-session by owner instruction. The codex WIP was preserved as `9a3f71cf` and proved to have **never compiled** (9 errors, including both commands declared on the top-level `Commands` enum, which would have shipped `jit reconfigure`).
- gf2 investigated and **parked**: two v1.0-format applied records (`jit-default`, `sim-research`) that current `main` rejects. Left byte-identical to how it was found; only the `cargo-ci.sh` fix was committed there.

## What to do next

- [ ] Dispatch wave 10. Both `6a479c58` (capture) and `9a81ab12` (removal) are `ready`, but **sequence removal after capture** — capture is the recovery path for a repository holding a pre-release record, and deleting the migration before its replacement exists is destroy-before-protect. There is deliberately **no DAG edge** enforcing this (see Traps).
- [ ] Reinstall `jit` with `./scripts/install-jit.sh` before any gate run; `main` has moved since the last install.
- [ ] For `6a479c58`, use gf2 as the real-world acceptance test: `/home/vkaskivuo/Projects/gf2` has two drifted directory packages under `packages/`, with exactly 3 known divergences from `jit-default` (`namespaces.type.examples` adds `type:simulation`, used by 23 issues; `namespaces.component.examples` replaced with six gf2 crate names; `item_kinds.requirement.id-pattern` narrowed to `REQ-[0-9]+`) plus at least one in `sim-research`'s `.jit/gates.toml`. The dogfood fixture has no comparable drift.
- [ ] After `9a81ab12` lands, close `7793b8b7` if the conversion counter is gone.
- [ ] Reconcile `docs/reference/profiles.md`'s `## V1.0 lifecycle boundary` section against the reversed `D-8`. It still frames capabilities as "deferred to the post-1.0 profile epic" and that epic is now v1.0. `b3d92595` owns the rewrite (its REQ-01 requires the section gone), but the contradiction is live on `main` now.

## Traps — do not repeat these

- **Do not read an idle ping plus an unchanged worktree as an idle worker.** It cost ~40 minutes of duplicated work this session: the lead sampled the worktree after an `idle_notification`, saw no commits and no writes for 20 minutes, wrote the documentation itself, and the worker's own (better) version arrived shortly after. After dispatching rework, **wait for the completion report**.
- **Do not supply review findings one at a time when a shape changes.** Rounds 2–4 were one defect class — a rehearsal entry that misreports the profile it names — found in three places (`targets`, `status`, then the generated schema's prose). The reviewer has no memory across rounds, so completeness is the lead's job. What closed it was demanding a **field-by-occurrence-shape matrix** in one pass.
- **Do not defer a documentation truth to a future issue.** Round 1 failed exactly this way: the lead forbade documenting the new commands in `cli-commands.md`, deferring to `b3d92595`, and the reviewer failed it against `@/charter/D-13`. Round 3's fix was blocked by the same freeze. If a change makes a statement false, fix the statement in that change.
- **Do not treat a worker's "awkward fit" note as future work.** The worker reported that `ProfileTargetMaterialization` carries no owner; the lead filed it against `cb26de35`; the reviewer bound it to this issue's REQ-04 one round later.
- **Do not rely on the shell's working directory for anything.** Two commits intended for `main` landed on a worker branch this session because cwd persisted into a worktree after an earlier `cd`. Use `git -C <path>` explicitly, every time.
- **The `cargo-ci` build-lock defect is fixed — do not re-add a watchdog.** `flock -o` now closes the lock descriptor in the child, so a daemonising `sccache` can no longer hold it. If a Cargo command still appears to hang, check `lslocks | grep cargo-ci`: `WRITE*` is a waiter, bare `WRITE` is the holder.
- **Do not add a DAG edge from `9a81ab12` onto `6a479c58`.** `jit` refuses it: the cross-story edge forces dropping `42c8c9c1 → 7156f64b`, routing that issue's containment through another story's node and diverging its membership label. Sequence it in the wave plan instead.
- **Do not reinstall `~/.cargo/bin/jit` from a worker branch.** It is shared with the lead and other agents. The wave-9 worker correctly built a provenance-stamped binary inside its worktree and put it on PATH for one run instead.
- **Do not re-open a Done issue without re-validating.** Re-opening `7156f64b` left its dependents stored as Ready with an unmet dependency and `jit validate` failed with exit 4; `jit validate --fix` repaired it.
- **Do not expect `jit profile apply --dry-run` to match the lifecycle rehearsal shape.** It plans each selector's package alone (one entry per occurrence); reconfigure/upgrade rehearse one aggregate selection. Recorded against `cb26de35`.
- Prior handoffs' traps remain in force, except the shipped-v1 ones, which `9a81ab12` supersedes by deleting the boundary.

## Open questions needing invoker input

None. Both open escalations were answered this session: the epic moves to v1.0, and the record migration is a clean cut with no legacy.

One item is decided but worth restating rather than assuming: **gf2's recovery path is now `capture`**, since the migration that would have fixed it is being deleted rather than widened.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Next issues: `jit issue show 6a479c58`, `jit issue show 9a81ab12`
- Planning docs: `dev/active/c639cfb5-jit-profiles-complete/plan.md`, `breakdown.json`, `progress.json`
- Charter: `dev/vision/9db27a3a-charter.md` (D-8 reversed, D-14 amended, 2026-08-10)
- Wave 9 merges: `f6c422de`, `d5f25800`, `cda04fb8`; scope change `201c4f34`; wave close `19e3117b`
- Build-lock fix: `4ce92063` (this repo), `5c244cad` (gf2)
- Preserved codex WIP (never compiled): `9a3f71cf` on `worktree-agent-7156f64b`
- gf2 record backups: `/tmp` scratchpad `gf2-records-backup/` (session-local; gf2 itself is untouched)
