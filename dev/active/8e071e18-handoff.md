# Handoff — Issue-based development artifact layout and serviceable archival (8e071e18) — session 1

**Date:** 2026-07-25
**Session number:** 1
**Prior handoffs:** None.

## Current state

- Epic: `8e071e18` — state: in_progress, claimed by `agent:jit-execution-lead`
- Wave in progress: wave 2 of 13 (closing); wave 3 not yet dispatched
- Children summary: 79 interior issues — 6 done (wave 1 + wave 2), 73 backlog/ready. Bracket nodes `55fe8ec2` (planning) and `644bfae1` (breakdown) were already done before this session.
- Active claims: none outstanding once wave 2 closes; all six wave-1/wave-2 issues were claimed `agent:worker` and completed.
- Open escalations: none open. Two were raised and resolved by the owner this session (E1, E2 — see the progress file and Traps).
- Progress file: `dev/active/8e071e18-progress.json` (wave plan, per-issue status, `surfaced_pitfalls`, `escalations`)

## What just happened

- Read the reviewed plan (`dev/active/8e071e18-plan.md`) and authoritative manifest (`dev/active/8e071e18-breakdown.json`); computed 13 topological waves over the 79-issue impl interior, excluding the bracket nodes.
- Verified **zero intra-wave footprint conflicts** across all 13 waves from the manifest's declared `creates`/`touches` — the plan's ordered-writers property holds, so every wave can fan out fully.
- Wave 1 dispatched (`69dd4e36`, `4f9af089`, `30595d22`) in isolated worktrees. All three merged to main; cargo-ci + code-review green; closed Done.
- `30595d22` failed code-review round 1 on finding F1 (its `satisfies:REQ-08` label claimed the epic's citation-scan criterion while the task ships only the warning vocabulary). Escalated per policy item 4; owner approved dropping the label; round 2 passed.
- Wave 2 dispatched (`b3fc1a92`, `1f80212b`, `7d6d2783`). All three merged to main. Merged main verified green independently: **3883 tests, 0 failed**.
- `7d6d2783` deviated from its declared footprint deliberately and correctly: the skip landed in `domain/artifact_discovery.rs`, not `artifact_classifier.rs`. Accepted — see Traps.
- `1f80212b` was committed by the lead from its worktree after it went idle with complete, verified work; the diff was reviewed before commit.

## What to do next

- [ ] Confirm `1f80212b` cargo-ci + code-review results (job was in flight at session end), then close wave 2: `jit issue update <id> --state done` for `b3fc1a92`, `7d6d2783`, `1f80212b`, commit `.jit/` + progress file.
- [ ] Dispatch wave 3: `f88a16d9` (declare the issue-scoped area registry) and `8cd6eb73` (derive the archive directory name without a title fallback). Both `cargo-ci` + `code-review`; footprints disjoint (`touches 3` and `touches 1`).
- [ ] Wave 4 next: `6aa82c0b`, `b4d1a2c8`, `22c11e40`. Note `b4d1a2c8` carries only `repo-validate` — it renames the one title-slugged archive directory.
- [ ] Before wave 8, decide the archive-execution strategy (see Traps — the 24 `archive-sweep-*` issues cannot fan out in worktrees).
- [ ] Before the epic's own gates, reconcile `surfaced_pitfalls` in the progress file against the epic's 15 `[hard]` criteria per `lead-review-protocol.md`. P1 (CHANGELOG) is the one needing a decision.

## Traps — do not repeat these

- **Do NOT let a dispatch prompt omit a reporting contract.** Wave 1's three prompts told workers what to build but never told them to report. All three ended turns silently with uncommitted work; two needed lead pings and one needed a takeover. Every wave-2 prompt carried an explicit contract ("ending a turn without having sent me a message is a failure of the task"), and wave 2's workers sent unprompted interim status. Keep that paragraph in every prompt.

- **Do NOT infer a stall from an idle notification, and do NOT infer liveness from its absence.** Workers ping idle while queued on the host-wide `cargo-ci` flock, and `cargo-ci` buffers each step's output to `$WORK/<step>.out` and prints only at the end — so a worker's output file stays empty for an entire multi-minute run. The reliable signal is `ps -eo comm | grep -E '^(cargo|rustc)$'` returning nothing **while** a worktree holds uncommitted work with no commit. Poll that, not process names via `pgrep -f` (which self-matches its own wrapper).

- **Do NOT gate a command on `pgrep -f "<pattern>"` where the pattern appears in your own command line.** A wait loop written as `while pgrep -f "ai-review.sh"; do sleep 20; done` matches the shell executing that very loop, so it never exits. Two gate batches idled ~50 minutes producing zero bytes before this was spotted. If you must wait for an external process, match on the executable name (`ps -eo comm`) or on a PID file, never on a `-f` pattern that your own wrapper contains. Better: do not serialize gates by process-polling at all — run them in one sequential shell command, since `jit gate evaluate` already blocks until the checker returns. *(A global `PreToolUse` hook now blocks this pattern outright; if you hit the block, read its message rather than working around it.)*

- **Do NOT run gate batches in the foreground.** The Bash tool's timeout maxes out at 10 minutes, and a single `cargo-ci` run alone approaches that. A foreground batch of `cargo-ci` + two `code-review` runs was silently truncated after the first gate — exit code 0, no error, the two reviews simply never ran and their statuses stayed unchanged. The `code-review` checker's own timeout is 1800s. Always run gate evaluations with `run_in_background: true` and read the output file, and **verify the recorded gate status afterwards** rather than trusting the batch's exit code.

- **Note there are stale orphan `codex` processes on this host** (observed at 23h and 7h old, from earlier sessions). They will match any pattern-based liveness check for the review gates and make a fresh review look like it is already running. Check process age before concluding a gate is in flight.

- **Do NOT run `jit gate evaluate` without reinstalling the binary first.** The stale-binary guard rejects any verdict whose binary provenance is not exactly HEAD, and HEAD moves on every merge and every `.jit/` commit. Sequence is always: commit → `./scripts/install-jit.sh` → `jit --version` → evaluate. Three gate evaluations were wasted learning this.

- **Do NOT run gate evaluations in parallel.** They serialize on per-issue locks and lose results. Run them sequentially. `cargo-ci` additionally holds a host-wide flock, so a parallel wave's workers each wait their turn — budget ~10-15 min per run.

- **Do NOT start a `cargo-ci` run without clearing `target/debug/incremental` first.** The `incremental-state` check fails on any leftover incremental directory, and rust-analyzer or an ad-hoc `cargo test` recreates it. Always `rm -rf target/debug/incremental && CARGO_INCREMENTAL=0 ./scripts/cargo-ci.sh`. Two full runs (~15 min each) were lost to this.

- **Do NOT treat a manifest footprint as the true blast radius.** Declared footprints were accurate for *intent* but three of six workers legitimately widened: `69dd4e36` into `config.rs` (the declaration's correct home) and `archive_preview_cli_tests.rs` (forced — the scaffold now authors `[documentation]`, so fixtures that prepended their own table produced duplicate TOML keys); `4f9af089` into `config.rs`, `commands/archive.rs`, and the preview suite; `7d6d2783` into `artifact_discovery.rs` and two test files. The intra-wave conflict analysis must be re-run against *actual* diffs before merging, not only against the manifest.

- **Do NOT put the directory-skip in the classifier.** `classify_entry` only sees artifacts that already exist as inventory entries, so it can suppress a blocker but cannot satisfy REQ-01's "contributes no artifact entry" clause — that would be exactly the reporting-layer filter the issue prohibits. The skip belongs in `discover_archive_artifacts` before `or_insert_with`. The edge must be dropped with the entry: `unpreservable_parents` (`artifact_classifier.rs:1015`) and `validate_proposed_layout` (`:594`) both block on a Supported edge whose target is absent from the inventory, so keeping the edge just relocates the blocker onto the parent.

- **Do NOT let a task carry a `satisfies:` or `enforces:` label that reaches further than its own criteria.** This cost two code-review rounds and two escalations in two waves, and it is the single most likely thing to bite the next session.
  - `30595d22` was blocked because `satisfies:REQ-08` read as a claim to the whole citation-scan criterion while the task shipped only the warning vocabulary. `@/rule/coverage-preview` actually defines the label as many-to-one coverage credit ("credited by **some** issue in its dependency closure") and seven issues carried it — but the reviewer read it as a total claim.
  - `b3fc1a92` was blocked because `enforces:@/invariant/single-source-prose` was contradicted by stale adopter copies that a *different, later* issue owns, even though the reviewer confirmed all three of its own hard criteria were met.
  - Both were resolved by dropping the label, owner-approved. **Before dispatching any issue, check whether its labels claim more than its `## Success Criteria` deliver**, and raise it with the owner up front rather than after a ~30 min review round. Same-shape issues still ahead: `6aa82c0b` and `ef2bd13c` (both `satisfies:REQ-08`, both deliver one part), and every task carrying `enforces:@/invariant/single-source-prose` whose criteria stop short of the adopter docs.

- **Do NOT assume a "covered by a later issue" pitfall is inert.** P2 (stale adopter docs at `docs/reference/configuration.md:50` and `example-config.toml:21`) was correctly attributed to `58c56743` in wave 11 — and then blocked wave 2's `b3fc1a92` anyway. From this session until `58c56743` lands, the adopter reference claims 3 managed paths while `jit init` writes 8. Expect any issue touching that fact to trip on it.

- **Do NOT plan to fan out wave 8's 24 `archive-sweep-*` issues into worktrees.** Each runs `jit archive container <id> --execute`, which mutates `.jit/issues/*.json`, `.jit/index.json`, and appends to `.jit/events.jsonl`. Twenty-four branches diverging on those files will not merge. They must run sequentially on main. They carry only `repo-validate` by design (D-20), so per-issue adversarial review is explicitly out of scope.

- **Do NOT assume `jit init` overwrites an existing `.jit/config.toml`.** It preserves it verbatim (`resolve_init_config`, `crates/jit/src/commands/init.rs:245`). Test fixtures that author a config *before* calling `initialize_fresh_repository` rely on this, and it is what keeps `4f9af089`'s `development_root = "workspace"` fixture honest after `69dd4e36`'s scaffold change landed.

## Open questions needing invoker input

- Question: should the epic add a CHANGELOG entry for its user-visible shipped-behaviour changes?
  - Context: `jit init` now authors a live `[documentation]` policy; archival classifies out-of-root artifacts as permanent and skips directory link targets. No epic issue owns a CHANGELOG entry — `6e963a7f` touches `CHANGELOG.md` but is scoped to citation-path repointing only. Repository convention records user-visible behaviour changes there (e.g. commits `b36f23aa`, `359f32cf`).
  - Options: (A) lead creates one epic-wide CHANGELOG task in a late wave; (B) fold it into `6e963a7f`'s scope; (C) no entry — none of the epic's 15 `[hard]` criteria requires one.
  - Recommendation: (A). Creating a leaf task inside the epic is within the lead's autonomy, but it adds a node the reviewed breakdown manifest did not sketch, so flagging it rather than doing it silently. Recorded as pitfall P1.

## Reference artefacts

- Epic: `jit issue show 8e071e18`
- Plan: `dev/active/8e071e18-plan.md` (criterion approach table, shared architectural contracts, generated decomposition overview, risk/decision table)
- Authoritative manifest: `dev/active/8e071e18-breakdown.json` (79 entries with `depends_on`, gates, and declared footprints)
- Investigation: `dev/active/8e071e18-investigation.md` (consumer inventory, blocker censuses, in-content citation census with `path:line`; its addendum supersedes earlier sections where they disagree)
- Progress file: `dev/active/8e071e18-progress.json`
- Bracket nodes: `55fe8ec2` (planning, done), `644bfae1` (breakdown, done)
