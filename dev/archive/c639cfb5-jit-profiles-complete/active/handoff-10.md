# Handoff — Complete profile lifecycle, composition, and upgrades (c639cfb5) — session 10

**Date:** 2026-08-11T16:20:00+03:00
**Session number:** 10
**Prior handoffs:** `handoff.md` through `handoff-9.md` in this directory

## Current state

- Epic: `c639cfb5` — state: backlog
- Wave in progress: wave 13 of 20 (waves 10, 11, 12 all closed this session)
- Children summary: 17 done, 0 in progress, 12 backlog/ready, 0 rejected (29 members after `cf42d08b` was added)
- Active claims: none for this epic. Three leases exist repo-wide from other agents (`bcff56be`/`agent:codex-profile-acceptance` and two others) — not this epic's, leave them.
- Open escalations: none
- Main head: `9e6fb8aa`; implementation merges this session `eb02a817` (capture), `d2c8f058` (shipped-v1 removal), `eabfd988` (exchange)
- Progress file: `dev/active/c639cfb5-jit-profiles-complete/progress.json`
- Next ready: `cf42d08b` (wave 13) and `f2389b18` (wave 14)

## What just happened

- **Wave 10 `6a479c58` done** (capture) after one rework round. `package_assembly.rs` (959 lines, test-gated, hardcoded) replaced by `profile/package_capture.rs` plus a pure `repository_state/package_tree.rs` producing one `RepositoryDelta`. Superseded paths deleted in the same change: the example, both scripts, its contract test, and `publish_staged_directory_noreplace`. `.github/workflows/release-artifacts.yml` cut over to `stage/jit profile capture`.
- Round 1 failed on a **real TOCTOU**: `read_confined` checked a pathname (`symlink_metadata`) then reopened it (`fs::read`); the executable screening read from the pre-check metadata, so the same race defeated the undeclared-executable refusal too. Fixed by one handle-anchored no-follow open, with `profile/nofollow.rs` extracted and shared with `package.rs`, which shrank 66 lines.
- **Wave 11 `9a81ab12` done** (shipped-v1 removal) first try. `+128/−2490`: decoder, conversion, 832-line pinned fixture, authentication capture phase, `converted_records` event field, conversion counter. Readers converged on the shared `read_applied_record`.
- Dispatched to **codex gpt-5.6-luna**. Its Rust work was good; two collateral effects, both reverted before commit — see Traps.
- **Wave 12 `15cc28c5` done** (pack/add) after two rework rounds, and **`42c8c9c1` story checkpoint done**.
- Round 0 failed on the archive **entry bound**: its doc asserted "no more than one directory per file", which is false, and `is_safe_relative_path` caps neither depth nor length — so `pack` produced archives `add` refused. Fixed by not writing directory entries at all, making entries `files + 1` regardless of path shape, with the byte bound rederived (path lengths sum to less than the manifest's size, itself inside `MAX_PROFILE_PACKAGE_BYTES`).
- Round 1 failed on **REQ-03**: directory entries admitted at any mode. This was a lead misjudgement — the worker disclosed the change, the lead accepted it reasoning that a directory has no declared mode to be unexpected against, and the reviewer's literal reading of the criterion governed. See Traps.
- The round-2 **whole-class sweep found a third defect no reviewer cited**: the archive metadata entry's mode was never checked at all, present since the first submission. All three mode classes now reach one `require_entry_mode`.
- **`42c8c9c1` round 1 failed** on a stale comment (`profile_apply.rs:1836`, "replaced after an authenticated v1 conversion") that wave 11's sweep and its code-review both missed. Fixed both instances of that class; round 2 passed with 0 findings.
- **Created `cf42d08b`** — a deep package packs but cannot be published (`max_paths: 8 * MAX_PROFILE_PACKAGE_FILES` assumes a path shape the model does not constrain; 511 assets at 9 components needs 5114 paths against 4096). `capture` fails identically, so it predates the exchange surface. Filed at v1.1; **the owner corrected that fixing bugs found in our own work is our responsibility, not scope expansion**, so it is now `milestone:v1.0` epic work, wired `15cc28c5 → cf42d08b → f6a17e0d`, as wave 13.
- **Created `4b7c06d0`** — the workspace test suite runs in under 30 s, a hard standing limit, at the owner's instruction. Blocks the v1.0 tag via `bb03df0a`.
- **Created `5f71a48d`** — `jit issue reject --reason` with prose leaves the repository failing validation, and neither `--remove-label` nor `validate --fix` recovers it. Blocks the v1.0 tag.
- **Rejected `7793b8b7`** as obsolete: wave 11 deleted both the conversion counter and the test asserting on it.
- Timing collection started at the owner's request: `scratchpad/timed.sh` → `timings.tsv`. This session spent **90.3 minutes** of measured gate wall-clock — cargo-ci 46.2 min, code-review 25.2 min, merged-tree gate 12.2 min.

## What to do next

- [ ] Dispatch wave 13: `cf42d08b`. It is `ready`. Design decision is open and belongs to whoever owns the package model — see Open questions.
- [ ] Reinstall `jit` with `./scripts/install-jit.sh` before any gate run whenever `crates/` has moved; `dev/` and `.jit/` changes do not invalidate the binary (`crates/jit/src/domain/binary_build_inputs.txt` is the authority, 34 entries, no `dev` entry).
- [ ] Dispatch wave 14 after it: `f6a17e0d` (story checkpoint, lead-run gates) and `f2389b18` (validate). `f2389b18` is already `ready` and does not depend on `cf42d08b`, so it may be dispatched in parallel with wave 13 if worktree isolation is respected.
- [ ] Before `f6a17e0d`'s and `dd268d0f`'s holistic-review gates, reconcile `surfaced_pitfalls` against their criteria per the review protocol. The `42c8c9c1` reconciliation is recorded in `progress.json` under `container_reconciliations` as the worked example.
- [ ] Waves 15–20 unchanged: `96268a98` → `cb26de35` → {`b3d92595`, `6574f951`, `3dcce8a8`} → `d7558ef6` → `8021d507` → `dd268d0f`.

## Traps — do not repeat these

- **Do not trust the shell's working directory — it bit five times this session, once with real consequence.** A gate evaluated against main's checkout instead of the worktree (aborted by the stale-binary guard); `install-jit.sh` invoked as `./scripts/install-jit.sh` from main built *main*, because the script resolves its repo from its own location, not the caller's cwd; a `gate status` read the wrong tree; and a lead `progress.json` commit landed on the worker's branch, surfacing later as the only merge conflict. Use `git -C <path>`, name the worktree's own `scripts/install-jit.sh` by absolute path, and put an explicit `cd` inside every command rather than relying on a previous one.
- **`jit gate evaluate` memoises a verdict over declared inputs.** A re-run returns `Verdict: taken from run <id> over the same declared inputs` in 0 ms. `target/` is not a declared input, so clearing an incremental cache does not invalidate a failure caused by it. Use `--force`. A stale `failed` verdict otherwise reads as broken work.
- **`codex exec` blocks reading stdin.** `codex exec -m … "prompt"` prints "Reading additional input from stdin..." and hangs until killed, producing nothing — one 15-minute survey was lost to this. Always append `< /dev/null`.
- **Do not let codex apply a documentation sweep to `dev/active/**`.** Dispatched for `9a81ab12` with an instruction to "grep the whole tree" for stale migration prose, it rewrote seven handoffs, `investigation.md`, `c639cfb5-research.md` (201 lines), `breakdown.json`, and `progress.json` — editing dated session records so they no longer mentioned a mechanism that existed when they were written, deleting concrete facts. Reverted in full. Exclude `dev/active/**` explicitly in every doc-sweep instruction; those files are history, not stale documentation.
- **`codex exec -s workspace-write` is the wrong sandbox for this repository.** It makes `.git` read-only, so the agent cannot commit and invents workarounds for its own uncommitted state — it weakened `tests/provenance_contract/repository_inventory.rs` with a `symlink_metadata(...).is_ok()` filter to tolerate deleted-but-uncommitted files. It also denies socket binding (15 unrelated test failures) and leaves the home cargo cache read-only. Either grant it a writable `.git`, or keep validation and commits lead-side as was done for wave 11.
- **A criterion that enumerates properties admits no implicit carve-out.** `15cc28c5` REQ-03 lists "absolute path, parent-directory traversal, symlink, or unexpected mode" as properties to refuse. The lead accepted an implementation that stopped checking mode for directory entries, reasoning the property had no referent there. The reviewer read the enumeration literally and failed it, costing a full rework round. The worker's framing is the durable one: "no directory mode is ever published" describes what the current code does with a value, while the criterion governs what the archive is allowed to *say*. When a change stops checking an enumerated property for any subset of inputs, amend the criterion through escalation or comply — do not judge the property inapplicable.
- **Demand the whole finding class, not the cited line — it finds defects reviewers miss.** Round 2 of `15cc28c5` swept for siblings of the cited mode defect and found the archive metadata entry's mode had never been checked at all, present since the first submission and cited by no reviewer across three review passes. Same pattern closed the `9a81ab12` stale-comment class in one commit.
- **Sweep on the removed mechanism's own vocabulary, not the phrases you expect.** Wave 11's Tier 2.5 sweep grepped `migration boundary|compatibility path|legacy record|five-field|pre-canonical` and passed. It missed `authenticated v1 conversion` at `profile_apply.rs:1836`, which the container holistic review then failed on. Grep for the domain verbs — convert, conversion, authenticate, migrate, v1.
- **`jit issue reject --reason "<prose>"` breaks repository validation.** The reason becomes a `resolution:<reason>` label, prose violates `@/invariant/label-format`, and the command warns but writes anyway. `--remove-label` then exits 0 without removing it, and `jit validate --fix` reports "no fixes needed" while `jit validate` fails. Recovery required hand-editing `.jit/issues/<id>.json` plus `jit recover`. Use a kebab token (`resolution:obsolete`) and put prose in the description. Tracked as `5f71a48d`.
- **An issue with no dependency edge fails repository integrity.** Filing `4b7c06d0` and `5f71a48d` "standalone, no epic" broke `jit validate` with "Found 2 isolated issue(s)". Every issue must be wired; `bb03df0a` (cut the v1.0.0 tag) is the anchor for v1.0 work outside a specific epic.
- **A clean merge still needs the merged tree gated.** Wave 12's branch and main both changed `commands/profile.rs` and auto-merged without conflict; per-issue gates each ran on a tree predating the combination. The merged-tree `cargo-ci` (735 s) is the only check that observes it. Skip it only when `git diff <gated-commit> HEAD -- <build inputs>` is empty.
- **Recorded gate durations include flock wait.** `cargo-ci` at 742 s had queued behind another run on `/tmp/cargo-ci.lock`. Do not read a gate's `duration_ms` as compute time when runs contend.
- **Do not let the worker and the lead both run the full `cargo-ci`.** That duplicated ~12 minutes per rework round. Standing instruction now given to workers: run only your added tests filtered, `cargo clippy --workspace --all-targets`, and `cargo fmt --all --check`; the lead's gate is the single full run.
- Prior handoffs' traps remain in force. The shipped-v1 ones are superseded — `9a81ab12` deleted that boundary.

## Open questions needing invoker input

- Question: Which way should `cf42d08b` bound package-tree publication?
  - Context: `publish_package_tree`'s `CaptureBudget { max_paths: 8 * MAX_PROFILE_PACKAGE_FILES }` assumes a path shape the package model does not constrain, so a deep package packs but cannot be published, and `capture` fails identically.
  - Options: (A) cap path depth or path length in the package model, which bounds the budget by construction but changes what a package is and would reject manifests that validate today; (B) derive the budget from total path bytes, which yields a number in the millions and stops being a budget; (C) make the closure enumeration not need a per-path budget.
  - Recommendation: (A). The repository is greenfield with no adopters (`@/inv/canonical-cutover`), so narrowing what a package may declare costs nothing today and makes the bound provable — which is the same shape that fixed the archive bounds. The specific cap is a routine implementation choice; the constraint is that it must not reject `profiles/jit-dogfood` or `profiles/jit-default`.

- Question: Should the `15cc28c5` limits CLI round trip be trimmed now?
  - Context: `test_profile_pack_and_add_carry_a_package_at_the_model_limits` publishes 512 files totalling 4 MiB through the transaction and costs ~33 s alone, over the whole standing budget `4b7c06d0` sets for the suite. It was added at lead instruction and proves the round trip the bound defect broke.
  - Options: (A) leave it and let `4b7c06d0` handle it; (B) trim it now to 512 small files, which still exercises the entry count and the real publication path while the unit test keeps the 4 MiB byte-bound proof.
  - Recommendation: (A), because (B)'s substance is already recorded against `4b7c06d0` as a concrete proposal, and reopening a passing issue for it would be a third rework round. The bound that broke was the entry count, not the byte payload — that is the key fact for whoever trims it.

## Reference artefacts

- Epic: `jit issue show c639cfb5`
- Next issues: `jit issue show cf42d08b`, `jit issue show f2389b18`
- Planning docs: `plan.md`, `breakdown.json`, `progress.json`, `investigation.md`, `c639cfb5-research.md` in this directory
- Threat model backing `15cc28c5`'s precheck security-review: `dev/active/15cc28c5/15cc28c5-threat-model.md` — records that the archive digest establishes integrity, not authenticity, by owner decision, and that no documentation may describe it as proof of origin (binds `b3d92595`)
- Charter: `dev/vision/9db27a3a-charter.md`
- New issues outside this epic: `4b7c06d0` (30 s suite budget), `5f71a48d` (reject/label defects), both blocking `bb03df0a`
- Timing harness: `scratchpad/timed.sh`, log at `scratchpad/timings.tsv` (session-local; re-create if needed)
