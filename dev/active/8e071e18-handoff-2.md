# Handoff — Issue-based development artifact layout and serviceable archival (8e071e18) — session 2

**Date:** 2026-07-26
**Session number:** 2
**Prior handoffs:** `dev/active/8e071e18-handoff.md` (session 1). Its Traps section remains in force except where this handoff records a resolution.

## Current state

- Epic: `8e071e18` — state: in_progress, claimed by `agent:jit-execution-lead`
- Waves 1–4 are **complete**. Wave 5 not yet dispatched.
- Children summary: 80 interior issues — 12 done, 68 backlog/ready. (79 from the manifest plus `fd88adda`, created this session.) Bracket nodes `55fe8ec2` and `644bfae1` were done before session 1.
- Active claims: none.
- Open escalations: none open. Two were raised and resolved by the owner this session (E3, E4).
- Progress file: `dev/active/8e071e18-progress.json`

## What just happened

- Resumed at wave 3 from session 1's handoff and progress file.
- **E3 (owner-resolved, standing):** owner granted standing approval to drop a `satisfies:`/`enforces:` label at dispatch whenever the issue's own criteria deliver only part of the target criterion, provided a sibling still credits it. Applied to `8cd6eb73` (dropped `satisfies:REQ-07`, `satisfies:REQ-13`), `6aa82c0b` (dropped `satisfies:REQ-08`), `22c11e40` (dropped `satisfies:REQ-13`; kept `satisfies:REQ-03` and `enforces:@/invariant/domain-agnostic`).
- **P1 closed:** owner decided the epic ships **no CHANGELOG entry**. No task created; `6e963a7f` stays scoped to citation-path repointing.
- Wave 3 dispatched: `f88a16d9`, `8cd6eb73` (worktrees, Opus). Both merged; three merge commits each verified to build in isolation.
- `8cd6eb73` passed `cargo-ci` + `code-review` first time.
- `f88a16d9` passed `cargo-ci`, **failed `code-review`** on F1 (blocking): the new `issue_scoped_areas` key was absent from `docs/reference/configuration.md`, citing `@/charter/D-13` and `@/invariant/single-source-prose`.
- **E4 (owner-resolved):** root cause was structural — the adopter config reference had been stale since wave 1 (3 managed paths documented vs 8 written; `permanent_paths = ["docs/"]` vs 6 entries) and is not repaired until `58c56743` in wave 11, which cannot move earlier. Same root cause had already blocked `b3fc1a92` (E2), meeting escalation policy 5a. Owner chose a remediation task.
- Created `fd88adda` (task, `story:area-classification`, `doc-review` + `docs-mechanical`), dispatched on main, landed both adopter docs synced to the shipped policy. Passed both gates first time.
- `f88a16d9` `code-review` re-run **passed**, reviewer stating the prior finding is resolved in the current tree, zero new findings. Wave 3 closed.
- Wave 4 dispatched: `22c11e40`, `6aa82c0b` (worktrees, Opus); `b4d1a2c8` executed by the lead directly on main (see N1 below). All three closed with every gate first-time green; both code-reviews returned zero findings.
- `b4d1a2c8` done: `dev/archive/a9b5dd08-use-strategic-label-slugs-in-archive-directory-n` → `dev/archive/a9b5dd08`, marker byte-identical, document link repointed, preview resolves a single destination, `repo-validate` passed.
- **Host root filesystem hit 100% full (0 bytes free)** mid-wave, silently failing `cargo-ci`'s `provenance` and `budget` steps with empty diagnostics. Lead reclaimed 153G of `target/` across 37 stale agent worktrees plus 1365 leftover scratch dirs in `~/.cache/jit-cargo-ci-tmp`. Disk now ~154G free.
- Merge of `6aa82c0b` + `22c11e40` broke the test build (`DocumentationConfig` gained a field; the resolver's test helper constructs that struct). Fixed by the lead in `8afdb018`. Recorded as N3.

## What to do next

- [ ] Dispatch wave 5: `f2cac2bd` (`repo-validate` only), `3559ca25` (`cargo-ci` + `code-review` + `mcp-ci`), `2ac35e8e` (`cargo-ci` + `code-review`). Re-check their labels against the E3 standing rule before dispatch.
- [ ] Before wave 8, decide the archive-execution strategy — the 24 `archive-sweep-*` issues cannot fan out in worktrees (session 1 trap, still in force). N1 below records the precedent the lead used for `b4d1a2c8`.
- [ ] Before the epic's own gates, reconcile `surfaced_pitfalls` against the epic's 15 `[hard]` criteria per `lead-review-protocol.md`. P2, P4, P5, P6 all need verification at their owning waves; P7 needs a disposition call.

## Traps — do not repeat these

All session-1 traps remain in force. Read `dev/active/8e071e18-handoff.md`. The ones below are new or newly sharpened.

- **Do NOT assume a merged branch is frozen.** Three workers this session committed *after* sending a "complete" final report. One landed while `install-jit.sh` was running and produced a `dirty=true` binary; another landed after a merge, so main was missing a commit and a running gate batch had to be aborted, rebuilt, and restarted. A gate verdict is rejected unless the installed binary's provenance is exactly HEAD with `dirty=false`. **"Complete" and "frozen" are different states — ask for the second explicitly**, and re-check `git log <branch> ^HEAD` after every merge.

- **Do NOT trust `scripts/verify-commit-builds.sh` to catch test-only breakage.** It runs `cargo build --workspace`, which never compiles `#[cfg(test)]` items. This session, `6aa82c0b` added a field to `DocumentationConfig` while `22c11e40` constructed that struct in a test helper; each branch was green alone, the merge did not compile its test code, and the verifier still exited 0. The break was caught only by an editor diagnostic. **After merging a wave, run an actual `cargo test` compile before gating** — the merge-commit verifier is necessary, not sufficient.

- **Do NOT let the Bash tool's working directory persist across a worktree visit.** The tool's cwd persists between calls. After a `cd` into a worktree, two later commands ran there unnoticed: one wrote a lead progress-file update into a worker's worktree (silently reverting an entry, since that branch predated it), and a worker lost two verification runs the same way. **Use absolute paths, or prefix every command with an explicit `cd /home/vkaskivuo/Projects/just-in-time &&`.**

- **Do NOT diagnose a `cargo-ci` failure without checking `df /` first.** `provenance` and `budget` are the only two steps needing multi-GB scratch, and on a full disk they fail with **empty** diagnostic files — exit 101/1 with no compiler error and no failing test name. That signature is failed writes, not failed assertions. 39 agent worktrees had accumulated ~170G of `target/`. **48 worktrees are still registered and nothing prunes them; this will recur.** Reclaim with `rm -rf .agents/worktrees/agent-*/target` for non-live workers (`target/` is gitignored at `.gitignore:2`, so nothing tracked is at risk) and `rm -rf ~/.cache/jit-cargo-ci-tmp/*`, after confirming `ps -eo comm | grep -cE '^(cargo|rustc)$'` is 0.

- **Do NOT wire a remediation task onto the issue whose review it unblocks.** `fd88adda` was first given `depends_on f88a16d9`, which made it `backlog` — blocked by an issue that was `in_progress` precisely because it was waiting on `fd88adda`. `jit issue claim` refuses a backlog issue. The code was already merged on main, so the real prerequisite was satisfied; rewiring onto `69dd4e36` (Done) made it `ready`. **Check the resulting state after `jit dep add`, not just that the command succeeded.**

- **`jit dep remove` does not exist — it is `jit dep rm`.** The error message says so, but the failed call still returns a JSON envelope that looks superficially like success if you only read the tail.

- **Do NOT expect `rg` to search `.jit/`.** It is a hidden directory, so ripgrep skips it by default. A citation census that uses `rg` alone will report a path as unreferenced while `.jit/issues/*.json` and `.jit/events.jsonl` still name it. Use `grep -r` for `.jit/`.

- **A `satisfies:` label is safe on the issue that closes a criterion's LAST clause, not its first.** `b4d1a2c8` kept `satisfies:REQ-07` and passed, because by the time it ran, `8cd6eb73` had already made the criterion's other clause true at HEAD. The E3 rule is about partial delivery *at review time*, not about many-to-one coverage in the abstract.

- **Do NOT hand-copy a shipped constant into adopter docs without a citation back to it.** That is what made the config reference go stale from wave 1 to wave 3 and cost two review rounds (E2, E4). `fd88adda` mitigated it by naming `SHIPPED_DOCUMENTATION_POLICY` in `crates/jit/src/config.rs` as the single source, which satisfies the invariant's "by citation" half. The mechanism is still unguarded — see P7.

## Open questions needing invoker input

None.

Resolved during the session, recorded here because the reasoning governs later waves: the question of whether to pre-emptively document a shipped configuration key whose adopter documentation belongs to a later-wave issue answered itself. `6aa82c0b` added `citation_scan_roots` and `code-review` passed with **zero findings**, so no pre-emption was needed. The distinction that matters:

- `issue_scoped_areas` is written into every new `.jit/config.toml` by `jit init`. An adopter is handed a key they cannot look up — that is the gap F1 blocked on.
- `citation_scan_roots` is optional and the scaffold deliberately does not emit it, so no adopter receives it undocumented.

So the trigger is **scaffold emission, not key existence**. A future issue adding a key the scaffold emits should expect F1; one adding an optional non-emitted key should not. `ef2bd13c` still owns documenting `citation_scan_roots` in wave 6, and its dependency on `2ac35e8e` looks conservative rather than necessary — all three of its criteria are already satisfiable — but nothing forces the question now.

## Reference artefacts

- Epic: `jit issue show 8e071e18`
- Session 1 handoff: `dev/active/8e071e18-handoff.md` (traps still in force)
- Plan: `dev/active/8e071e18-plan.md` (criterion approach table, shared contracts, decomposition overview)
- Authoritative manifest: `dev/active/8e071e18-breakdown.json` (79 entries with `depends_on`, gates, declared footprints; `fd88adda` is not in it)
- Investigation: `dev/active/8e071e18-investigation.md` (addendum supersedes earlier sections on disagreement)
- Progress file: `dev/active/8e071e18-progress.json` (wave plan, per-issue status, `escalations` E1–E4, `surfaced_pitfalls` P1–P7, `lead_notes` N1–N2)
- Key name fixed this session: `citation_scan_roots` (`crates/jit/src/config.rs`) — `ef2bd13c` must document this exact name.
- New public surface: `resolve_artifact_directory` and `ArtifactDirectoryError::UndeclaredArea` in `crates/jit/src/domain/artifact_directory.rs`; `archive_container_slug` is `pub(crate)` so that module can reuse it.
