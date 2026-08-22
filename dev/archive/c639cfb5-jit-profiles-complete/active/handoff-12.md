# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 12

**Date:** 2026-08-13T17:00:00+03:00
**Session number:** 12
**Prior handoffs:** `handoff.md`, `handoff-2.md` … `handoff-11.md` in this directory

## Current state

- Epic: `c639cfb5` — state: in_progress (this session moved it out of backlog)
- Wave in progress: wave 16 of 20, not yet dispatched
- Children summary: 20 done, 0 in progress, 10 backlog/ready, 0 rejected (31 members, up from 29 — two created this session)
- Active claims: `0708d692` claimed `agent:worker` (out-of-epic bug, work complete, gates not finished)
- Open escalations: none awaiting a reply. Seven owner decisions this session are recorded in `progress.json` → `escalations` and `scope_changes`
- Progress file: `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Main head: `98d51ab6b`, working tree clean, `jit validate` passes
- Next ready: `5a3ecba0`, then `cb26de35` (deliberately serialized behind it — see What to do next)

## What just happened

- **Wave 15 `96268a98` done and merged**, review PASS on round 2. Adds `jit profile diff`. Merged at `1ab0255bc`; merged tree gated warm (4512 tests, suite-clock 22,669 ms).
- Round 1 FAILED. The `code-review` gate found that REQ-01's "targets **and identities**" was unmet: the report stated file targets only, and `compose_profile_contributions` returned `Err(ContributionCompositionConflict)` so a semantic conflict could never reach a report at all. **The worker had classified this as an interpretation** — "REQ-01's 'targets and identities': interpreted as targets named canonically" — which is the softening move `lead-review-protocol.md` forbids. The lead had primed that failure by writing a binding determination about *counting* conflicts and never questioning the noun.
- Lead added three findings of its own to close the round completely: REQ-02's owner naming must extend to identities; `difference.rs`'s module doc asserted something true only of the file half; and the worker's surviving-region-owner change (`Retain` replacing `Remove`) altered epic REQ-07's removal rule with no test pinning it. All four closed at round 2, verified by the lead reading HEAD rather than trusting the worker's account.
- **Verified the epic's motivating use case end to end against a built binary**, prompted by the owner asking how asset modification works. Result across the four classes of profile-owned content, all measured:
  - live-source asset (`assets/live/…`): edit → `capture` → `apply` → `validate` clean. No version bump; `apply` carries no installed-identity precondition, unlike `reconfigure`.
  - install-only asset (`assets/install/…`): `capture` reports `unchanged`; `apply` refuses with a conflict; the edit survives.
  - managed region: `capture` no-op; `apply` **silently discards** the edit and re-renders. Owner ruled this intended — regions are profile-owned territory.
  - semantic contribution: `capture` no-op; `apply` and `reconfigure` both report "unchanged"; `jit profile validate` exits 4 and `jit validate` fails with a `profile-ownership` rule error **permanently**, `--fix` offers nothing. No sanctioned sequence clears it — verbatim the epic Background's description of the MVP trap, still true for this class.
- **Owner decided the refresh loop must cover every class the repository authors**, and challenged reading region bodies back since that content is generated. The lead generalized that principle: refresh applies where the *repository* authors the content. Live-source assets (works) and contributions (broken) qualify; regions and install-only assets are package-authored and have no repository-side value to read back.
- **Created `5a3ecba0`** (contribution refresh on capture), now 7 hard criteria after two owner-approved additions. **Created `acf49914`** (end-to-end edit-and-refresh regression test — the motivating loop had no test joining capture's half to apply's half).
- **Created and dispatched `0708d692`** (out-of-epic, under `epic:v1-release`) after the owner said the gate must be fixed before proceeding. Worker complete, `cargo-ci` passed, all five criteria verified by the lead. Round 1 FAILED the lead's Tier 2.5 sweep on one stale site — `dev/TESTING.md`'s "Incremental compilation" bullet still stated the deleted predicate, the exact belief that generated the `rm -rf target/debug/incremental` folklore. Fixed at `ba84bb960`; the worker's own sweep then found a second site, `CHANGELOG.md`'s `jit:57d0eb79` entry, and fixed it on the reasoning that an entry still under `## [Unreleased]` has never shipped and would otherwise state both predicates in one release. The lead agrees with that call. Re-swept clean across `docs/**`, `dev/TESTING.md`, `CHANGELOG.md`, `README.md`, `AGENTS.md`, `CLAUDE.md`, `contrib/**`, `crates/**`, `scripts/**`. **`code-review` was not run and the branch is not merged — the owner assigned both to the next session.**
- **`0708d692` corrected the lead's diagnosis.** The issue blamed first-touch page-cache I/O. The worker measured: `cargo nextest` runs setup scripts inside the suite clock, and `setup-recorded-failure-corpus.sh` selected `cargo test -p jit --test cli_issue`, so Cargo's feature unification resolved a second variant of the dependency graph and **compiled 60 crates, 24,677 ms, inside the measured clock** on every fresh target. Selecting `--workspace` makes it 131 ms / 0 units. Cold verdict went 57,019 ms FAILED → 26,832 ms passed; warm unchanged at ~22,600 ms.
- It also found the REQ-04 guard was itself broken: `test_cargo_ci_disables_incremental_compilation_before_the_first_step` **passed with the export commented out**.
- **Settled the REQ-12 live-asset question** left open by handoff-11, owner accepted. The "encoded twice" framing was wrong — evidence in `progress.json` → `surfaced_pitfalls`, entry "live-asset classification encoded twice".
- **Re-measured the gate incremental ban** on the owner's instruction. Verdict: keep it. Details in `progress.json` → `measurements`.
- **Retired the three-lock coordination protocol** on the owner's decision. The lead now takes no locks.
- **Rejected `7793b8b7`** as dissolved, reason appended to its description.

## What to do next

- [ ] **Run `0708d692`'s gates and merge it. The owner assigned this session's gates to you.** The branch `worktree-agent-0708d692` is clean at `d95bbfe4e`; the lead's blocking finding is closed and re-swept. `cargo-ci` already passed at `0ad02adb5` — **re-evaluate it, because three commits landed after that verdict** (`ba84bb960` docs, `d95bbfe4e` tracker state) and `jit gate evaluate` reuses a verdict over unchanged declared inputs, so pass `--force` if it returns `duration_ms: 0` / `derivation: "reused"`. Then evaluate `code-review`, merge to main, gate the merged tree, transition to done. Do not skip the merged-tree gate: it is the only check that sees the combination.
- [ ] Note for that review: the last two commits are documentation and tracker state only. The worker ran `./scripts/docs-mechanical.sh` rather than a full `cargo-ci` for them — M2/M5/M6/M7 pass; M3 reports `MISSING: .jit/config.toml:map-entry:namespaces:workflow`, which the worker verified is pre-existing by re-running with its edits stashed. That is dogfood namespace-registry drift, unrelated and outside this issue.
- [ ] **Everything else waits on `0708d692`.** The owner's instruction was "fix it now to proceed properly" — every remaining wave's gate runs through the script it repairs.
- [ ] Dispatch `5a3ecba0` (contribution refresh, 7 criteria). It is `ready`.
- [ ] **Hold `cb26de35` until `5a3ecba0` lands.** Owner decision: serialize. `cb26de35` unifies presentation across every profile subcommand including the `diff` output that just landed and whatever `5a3ecba0` changes about capture's reporting; it should standardize a surface that has stopped moving. Both also touch `commands/profile.rs`, `main.rs`, and `cli.rs` heavily.
- [ ] Then `acf49914`, then the `f6a17e0d` checkpoint. **`f6a17e0d` is deliberately wired behind `5a3ecba0`** — its REQ-01 names capture "from declared repository targets" and a package being "refreshed", so the contribution pitfall's subject is named by that criterion and must close before the container gate runs.
- [ ] Waves 17–20 unchanged: {`b3d92595`, `6574f951`, `3dcce8a8`} → `d7558ef6` → `8021d507` → `dd268d0f`.
- [ ] Answer the open question below about the three budget thresholds before the epic's own container gate.

## Traps — do not repeat these

- **The `flock` traps, the host-CPU lock, and the three-lock ordering are RETIRED.** Handoff-11's lock traps no longer apply: the owner retired host-wide locking on 2026-08-13 as too heavy-handed, and the peer session that motivated it (epic `4b7c06d0`) closed. Take no locks. `scripts/cargo-ci.sh` still takes `cargo-ci.lock` internally, which is the script's business. The rationale and the deadlock it prevented are preserved in `progress.json` → `coordination` in case a second session is ever run here.
- **The `incremental-preflight` / `rm -rf target/debug/incremental` trap is being retired by `0708d692` — do not reinstate it.** Once that merges, `incremental-baseline` records what exists before the first compilation and `incremental-state` fails only on what the run added. If you find yourself clearing that directory before a gate, check whether `0708d692` merged first. `CARGO_INCREMENTAL=0` on ad-hoc cargo invocations is still correct hygiene and the gate-run ban stays in force.
- **The cold-worktree suite-duration trap is being retired by the same issue, but not fully.** After the fix the cold margin is 3,168 ms (26,832 ms against 30,000 ms), and ~4.3 s of first-run fixture construction is still inside the clock. A cold suite-duration failure on a slower host remains possible. Read `dev/benchmarks/cold-warm-verdict-0708d692/README.md` before diagnosing one.
- **A criterion's nouns bind as tightly as its verbs.** The lead read REQ-01's "targets and identities" as a counting problem, wrote a binding determination about enumerating the complete conflict set, and never questioned "identities". The worker then resolved the ambiguity by narrowing the noun, and only the independent reviewer caught it. **When you write a binding determination, you are also telling the worker what not to think about.** Re-read the criterion after drafting one.
- **The shell's cwd does not survive between tool calls the way you expect, and `jit` resolves its repository from cwd.** This session created issue `5a3ecba0` into a *worker's* `.jit` because the shell was still inside that worktree; it then reached main only via the branch merge. Backgrounded commands inherit the session cwd, not the `cd` of a previous backgrounded command. **Put `cd <path> &&` in the same command as any `jit` or gate invocation**, and check the first line of a background job's output to confirm where it ran. Two gate evaluations were started against main by accident and had to be killed before they recorded a verdict against the wrong tree.
- **`jit issue update` has no `--reason` flag.** `jit issue update <id> --state rejected --reason "..."` fails with a usage error. Append the reason to the description first, then transition.
- **`jit dep add` refuses an edge that breaks transitive reduction.** Adding `acf49914 → 5a3ecba0` failed because `acf49914 → 96268a98` was already implied. Use `--reduce` to drop the redundant edge in the same operation.
- **`codex exec` hangs on stdin.** This session dispatched a read-only investigation of the REQ-12 question; it produced zero bytes in four hours and was killed. Pitfall 10 in `progress.json` already recorded this and the lead dispatched it anyway. **The REQ-12 finding therefore rests on the lead's own trace with no independent check** — noted in its record.
- **Do not let a documentation sweep touch `dev/active/**` or `dev/archive/**`.** Still in force. A prior session's sweep rewrote seven handoffs and was reverted in full. This session's Tier 2.5 sweeps hit those paths repeatedly and correctly left them; the rework instruction to the `0708d692` worker bounded its sweep explicitly for this reason.
- **`jit gate status` reads the `.jit` of the checkout you run it from.** A gate evaluated in a worker's worktree writes its record there, and querying from main reports "has not been run yet". Query from the same tree.
- **Every measurement in `dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md` is obsolete** (owner, 2026-08-13). Its baseline table records 92 GiB `target/`, 19 GiB incremental, 140 integration targets, 712 executables; the tree now reports 10 targets and 14 executables. **Five sites still cite it as their justification** — see the open question below. Do not cite it as current, and do not add new citations to it.
- Prior handoffs' traps otherwise remain in force. **Superseded from handoff-11: the `flock -o` trap, the three-lock protocol, the "gate a freshly merged worktree twice" remedy** (replaced by `0708d692`'s fix), **and the `find … -name incremental -not -empty` check** (already retired there, now removed from the gate entirely).

## Open questions needing invoker input

- Question: Should `MAX_INTEGRATION_TARGETS=12`, `MAX_EXECUTABLE_BYTES=2 GiB`, and `MAX_TEST_SUITE_SECONDS=30` be re-derived, and by whom?
  - Context: `scripts/rust-build-budget.sh:34-37` states outright that the justification for all three "live in the design doc's acceptance budgets, not in this file", citing `73482aa1-rust-build-efficiency.md` — which the owner has confirmed is obsolete. `scripts/cargo-ci.sh:152` and `dev/TESTING.md:288` cite it too. So three enforced constants currently rest on measurements that do not describe the tree, which is a `@/inv/single-source-prose` staleness defect at five sites. Current usage: 10/12 targets, 1.49 GiB of 2 GiB, ~22.6 s of 30 s.
  - Options: (A) fold into `0708d692` before it merges; (B) file a separate issue under `epic:v1-release`; (C) leave the constants and only repoint the citations at current evidence.
  - Recommendation: (B). `0708d692`'s worker was told explicitly not to re-derive them and its criteria do not cover it; widening it now would mean re-reviewing passed work. (C) is not sufficient on its own — there is no current evidence to repoint at until someone measures.

- Question: Is the 3,168 ms cold margin on the suite budget acceptable for v1.0?
  - Context: After `0708d692`, cold measures 26,832 ms against a 30,000 ms threshold. The residual is the suite's own first-run fixture construction (`target/jit-profiled-repository-fixtures`, `target/debug/jit-recorded-failure-corpus`), about 4.3 s, still inside the clock. REQ-01 asks that cold and warm reach the same verdict and they demonstrably do on this host — but by subtraction, not by construction.
  - Options: (A) accept and record; (B) move fixture construction outside the clock the way the duplicate compile was moved, as a follow-up.
  - Recommendation: (A) for now, (B) if any cold failure recurs. The worker itself raised this; it is recorded rather than hidden.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Next issues: `jit issue show 0708d692`, `jit issue show 5a3ecba0`, `jit issue show cb26de35`
- Planning docs in this directory: `plan.md`, `breakdown.json`, `progress.json`, `investigation.md`, `c639cfb5-research.md`
- Gate determinism evidence: `dev/benchmarks/cold-warm-verdict-0708d692/README.md` plus its `raw/` runs — cold and warm verdicts, the attribution of the cold penalty, and the deliberate suite-lengthening regression
- Obsolete but still cited: `dev/archive/6eb585bc-core-maintenance/active/73482aa1-rust-build-efficiency.md`
- Threat model binding `b3d92595`: `dev/active/15cc28c5/15cc28c5-threat-model.md` — the archive digest establishes integrity, not authenticity; no documentation may describe it as proof of origin
- Charter: `dev/vision/9db27a3a-charter.md`
