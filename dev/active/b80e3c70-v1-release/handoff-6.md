# Handoff — Version 1.0 release (`b80e3c70`) — session 6

**Date:** 2026-07-31
**Session number:** 6
**Prior handoffs:** `handoff-5.md`, `handoff-4.md`, `handoff-3.md`, `handoff.md` in this directory

**Main baseline before this session:** `fb42fb44`
**Main at session end:** `2782b63a`

## Current state

- Epic: `b80e3c70` — state: backlog, assigned `agent:jit-execution-lead`, release sink intentionally unmet
- Wave in progress: waves 4, 5 and 6 are complete; wave 7 (`bb03df0a`) is withheld by invoker decision
- Children summary: 24 done, 1 rejected, 1 ready (`bb03df0a`)
- Active claims: `bb03df0a` unclaimed; every other child released on completion
- Open escalations: none outstanding — both raised this session were answered
- Progress file: `progress.json` in this directory

## What just happened

- Reclaimed 11 merged worktrees across the session (8 from waves 2–3 at the start, 3 more after waves 4–5). `git worktree remove` was permitted this session after the invoker allowed it; 30 GB freed at the first pass. Preserved `agent-a122b9b3` (forensics), `lead-install-clean`, `steward-v1-readiness`.
- `f9af9788` — Done, gates first round, no rework. Split publication into `release-publish.yml` (tag trigger; calls `ci.yml` and `security-audit.yml` on the tagged commit, then `release-artifacts.yml`, then publishes) and `release-artifacts.yml` (reusable + its own `pull_request` trigger). Closed all three findings routed here by waves 2–3. Two new contract assertions with six case vectors: `jobs.<name>.uses` and `global.publication`. Merge `cc63796c`, F1 fix `18ee8c8e`, completion `299ad92d`.
- `c5c324df` — **created this session**, Done, gates first round. `profiles/jit-dogfood/manifest.toml` declared `jit = ">=0.2.1, <0.3.0"` against a 1.0.0 product; `crates/jit/src/profile/package.rs:253` only parses that requirement, so nothing rejected it and `jit profile list --json` published the contradiction. Range now `>=1.0.0, <2.0.0`, and `scripts/release-version-contract.py` reads that manifest and fails when the derived version falls outside it. Suite 30 → 33 tests. Merge `101bbee2`, completion `53afccb5`.
- `e90b9a3b` — Done after 2 rework rounds. Landed canonical homes (`INSTALL.md` split into published-archive vs source-checkout, `deployment.md` owning the one-image `/repo` deployment, new `docs/how-to/mcp-integration.md`, `release-policy.md` owning version/artifact/compatibility/MSRV procedures) and M6 of `docs-mechanical`: `scripts/docs-check-canonical.sh` + `scripts/docs-canonical-homes.toml`, binding each documented literal to the automation that keeps it true. Round 1 absorbed `080e54c6`'s pruning per invoker decision; round 2 restored source-checkout prerequisites the pruning had stripped. Merges `86e49239`, `682986b9`, `14516064`; completion `e372949d`.
- `080e54c6` — Done, gates first round, run as a verification pass rather than a rewrite. Found and closed two genuine gaps: README restated the storage format-version rule that `storage-format.md` owns, and `contrib/README.md` carried a `jit gate define` walkthrough duplicating `custom-gates.md`. Added the two bindings whose REQ-02 fact classes had no declared owner (the Compose command; one capability of the aligned supported set). Merge and completion `2782b63a`.

## Verified preconditions at session end

Re-derived at `2782b63a`, not carried forward from a prior handoff:

- `cargo audit -D warnings` — exit 0 over 386 crate dependencies
- `npm audit --omit=dev` — 0 vulnerabilities in `web/` and in `mcp-server/`
- `scripts/rust-version-policy.py` — `{"declared": "1.97", "stable": "1.97.1", "minor_lag": 0, "status": "within-window"}`
- `python3 scripts/release-version-contract.py` — exit 0, product version 1.0.0, now including the profile range
- `jit validate` — passes; `scripts/verify-commit-builds.sh` verified every merge commit this session

## What to do next

- [ ] `bb03df0a` is the only open child. It needs the maintainer: the push to the public remote, the annotated `v1.0.0` tag, and the clean-container verification. Do not cut the tag without an explicit instruction — see Open questions.
- [ ] Before tagging, re-derive the four preconditions above rather than trusting this file.
- [ ] After the release exists, check the rendered release body's four relative links (see Traps). Repair by editing the body or re-cutting the tag.
- [ ] Reinstall `jit-server` before any host-side release verification: the installed binary reports `0.1.0` while `crates/server/Cargo.toml` declares `1.0.0`. `scripts/install-jit.sh` installs `jit` only.
- [ ] Epic completion (Section 10) still owes: reconcile `surfaced_pitfalls` against the epic's criteria, run the epic's six gates, write the completion report, archive and link it.

## Traps — do not repeat these

- **Run review greps in the worker's worktree, not in main.** A `git grep` for a stale name was run from the main checkout while reviewing a worktree branch and produced a finding against main's own pre-merge state. The worker had already fixed it. Check `pwd` before every review grep; `git -C <worktree> grep` is safer than relying on shell cwd.
- **The leak check reports the lead's own gate evidence as a worker leak.** `check-leak-into-main.sh` diffs against the dispatch-time snapshot, which predates any gate the lead evaluates on main. Three `.jit/gate-runs/` entries and the modified `events.jsonl`/issue JSON showed up as "likely worker leaks" this session. Read the paths before recovering anything: `.jit/gate-runs/*` belongs to the lead.
- **A piped leak check cannot gate a merge.** `check-leak-into-main.sh | tail -3 && git merge …` runs the merge regardless, because the pipeline's status is `tail`'s. Run the check as its own command and read it.
- **Absorbing a successor's scope is a decision, not a remedy the lead may pick.** `doc-review` blocked `e90b9a3b` on a checker scope `080e54c6` owned, while `080e54c6` could not start until `e90b9a3b` was Done — a genuine deadlock, not an ordinary successor-collapse finding the standing rule covers. The invoker chose absorption over a scope-boundary amendment. Recognize the deadlock shape before reaching for the boundary note.
- **Pruning a walkthrough can strip prerequisites the surviving command still needs.** Removing the MCP install section left `node index.js` in `mcp-server/README.md` with no `jit`-on-`PATH` or dependency step behind it — a command that fails for any reader who follows it. After removing any walkthrough, check every command that survives in that file for prerequisites the removal carried away.
- **An idle notification is not a report and not a stall.** Workers pinged idle repeatedly with no commits, then committed minutes later. Check the branch and the worktree's dirty state before concluding anything; do not re-dispatch on a ping.
- Unresolved traps from `handoff-5.md`, `handoff-4.md` and `handoff-3.md` remain in force, in particular: run `jit` gate and status commands from the main checkout; verify a gate's `last_run_at` moved rather than trusting a chained command's exit status; amend an issue before evaluating gates, never after; keep `target/debug/incremental` empty before `cargo-ci`; verify a version pin against its upstream index before accepting it.

## Open questions needing invoker input

- Question: When should `v1.0.0` be cut, and by whom?
  - Context: The invoker confirmed this session that no tag is cut. `bb03df0a` needs a push to the public remote — main is now far ahead of `origin` and every workflow this epic built has still never executed on GitHub — plus the annotated tag and the clean-container verification.
  - Options: (a) maintainer pushes and tags, lead verifies the published release afterwards; (b) lead pushes on an explicit go-ahead, maintainer still tags; (c) defer the release act entirely.
  - Recommendation: (a), unchanged from session 5. Release authority stays with the maintainer, and the first real run of these workflows should be watched by the person who can delete the tag.

## Reference artefacts

- Epic: `jit issue show b80e3c70`
- Only open child: `jit issue show bb03df0a`
- Progress: `dev/active/b80e3c70-v1-release/progress.json`
- Prior handoff: `dev/active/b80e3c70-v1-release/handoff-5.md`
- Release workflows: `.github/workflows/release-publish.yml`, `.github/workflows/release-artifacts.yml`
- Contract grammar: `dev/workflow-contract.md`, `.github/workflow-contract.yml`
- Canonical documentation homes: `scripts/docs-canonical-homes.toml`, `scripts/docs-check-canonical.sh`
- Release note the publish job renders: `docs/release-notes/v1.0.0.md`
