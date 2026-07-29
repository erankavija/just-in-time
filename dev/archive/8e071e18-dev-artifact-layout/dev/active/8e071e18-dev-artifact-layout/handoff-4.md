# Handoff — Issue-based development artifact layout and serviceable archival (8e071e18) — session 6

**Date:** 2026-07-27
**Session number:** 6 (session 5 produced no handoff; its work is reconstructable from `git log` and the progress file)
**Prior handoffs:** `dev/active/8e071e18-handoff.md` (1), `dev/active/8e071e18-handoff-2.md` (2), `dev/active/8e071e18-dev-artifact-layout/handoff-3.md` (4). Their Traps remain in force except where this handoff records a resolution.

## Current state

- Epic: `8e071e18` — in_progress, claimed `agent:jit-execution-lead`
- **81 of 88 children done.** Waves 1–11 complete. 7 remain: the epic itself plus 6 children.
- Active claims: none outstanding. `4a6fb8c1` was set up and then released unstarted when the session was asked to wind down; its worktree is removed.
- Open escalations: **none.** E13–E17 were all raised and resolved this session.
- Progress file: `dev/active/8e071e18-progress.json` — waves, per-issue status, E1–E18, P1–P28, N1–N32

The six remaining children and exactly what each needs:

| issue | state | blocked on | what it needs |
|---|---|---|---|
| `4a6fb8c1` | ready, no unmet deps | nothing | **the unblocker.** Owns `docs/reference/cli-commands.md`. Must absorb three reviews' findings — see below |
| `58c56743` | ready, `doc-review` **failed** | `4a6fb8c1`'s edit | re-run `doc-review` only; `docs-mechanical` already passed |
| `0b3840f1` | in_progress, `code-review` and `doc-review` **failed** | `4a6fb8c1`'s edit | re-run those two only; its other four gates passed |
| `143388dc` | ready, no unmet deps | nothing | **can run now, independently.** 5 gates |
| `5e9305ec` | backlog | `58c56743` | dispatch after `58c56743` closes |
| `af264b06` | backlog | `4a6fb8c1`, `5e9305ec` | final checkpoint, 2 gates |

**`4a6fb8c1` must absorb findings from three separate reviews, all in its own file:**

1. `0b3840f1` doc-review F1 **and** code-review F1, raised independently: `docs/reference/cli-commands.md` documents neither `jit doc dir` nor `jit doc conformance`. Add entries under Document Commands covering arguments, output, JSON shape, no-write behaviour, and conformance's advisory/non-blocking semantics. These are its own REQ-05 and REQ-06.
2. `58c56743` doc-review F1 (blocking, cites `@/invariant/single-source-prose`): `cli-commands.md:316` still independently specifies the short-id/slug derivation. Replace the shared derivation with a link to `configuration.md#issue-artifact-directories`, retaining only archive-specific behaviour — archive root, markers, fallback handling. That is its REQ-07.
3. Its REQ-02 was amended under E13 and now names **retention**, not archival by copy. Read its Notes.

## What just happened

- **All 24 archival executions ran and closed** — the epic's central repair. 176 relocated, 19 mirrored, 83 retained, 0 blockers, 24 recorded execution events, then re-verified under one uniform assertion set including byte-for-byte hash comparison of 214 files against the plan that moved them.
- **`ca832358`** filed from the rehearsal and landed: a retained artifact was still relinked to a destination the run never writes (33 across 8 containers), which made `jit validate` fail after the runs. A/B over one unchanged state: actions identical, retained-with-reference-change 33 → 0.
- Closed this session: `84c4e956`, `334bcd6f`, `89b5899a`, `8ff4cff3`, `35499d1e`, `ca832358`, the 24 sweeps, `aa38b236`, `623e4c46`, `834a78a8`, the five verification-only citation issues, `aa5db9a1`, and the five disposition records.
- **`aa38b236`** produced the completeness record (138 files remain under the managed areas; per-reason 9 live owner / 3 commit-pinned / 4 owner outside every archived subtree / 122 no document reference) and the consolidated warning list (641 occurrences, 105 citing files, cited set exactly equal to the relocated set).
- **The five disposition records found six mismatches** against their own planning-time enumerations, all on mirrored rows — 22 named mirror destinations of which 20 hold no copy. See N32.
- **`623e4c46` and `834a78a8` pulled forward** out of turn: 7 stale citations were failing `docs-mechanical` for the whole epic. Rewired onto the sweeps that moved their targets.
- Five citation issues needed **no edit**: their cited artifacts were mirrored, so the sources stayed and the citations still resolve. Each verification is recorded in its claim commit.
- **The showcase alternate theme** (`gruvbox.css`) was reached through the mechanism per the owner's decision — reference added to `2fbd2a82`, that container's archival re-run, one artifact relocated into the existing destination and the reference relinked. Recorded in `disposition-showcase-theme.md`; it closes the last REQ-10 gap.
- Owner decisions: **E13** (two criteria still named the pre-E9 copy mechanism), **E15** (epic REQ-09 scoped to terminal issues outside the epic's own subtree), **E16** (`f9e42a43`, `f289ff18` filed), **E17** (reorder: documentation before checkpoints, after the fourth recurrence crossed MAX_SAME_FINDING_REPEATS).
- Confirmed the previously unverified half of N8: the stale-binary guard tolerates a moved HEAD when no build input changed.

## What to do next

- [ ] **Dispatch `4a6fb8c1` first** — it unblocks three issues. It is `ready` with no unmet dependencies; a worktree was created and removed, so make a fresh one. Give it all three findings listed in Current state above, plus the P28 worktree warning: a `MISSING:` line naming a `dev/` directory is that class, not its defect.
- [ ] **`143388dc` can run in parallel right now** — ready, no unmet dependencies, 5 gates. Nothing about it waits on `4a6fb8c1`.
- [ ] After `4a6fb8c1` lands: re-run `58c56743`'s `doc-review` and `0b3840f1`'s `code-review` + `doc-review`. Nothing else on either is outstanding.
- [ ] Then `5e9305ec`, then `af264b06`.
- [ ] **Before the epic's gates, reconcile P1–P28 against the 15 `[hard]` criteria** per `lead-review-protocol.md`. Still needing a disposition call: **P18** (an eligible plan can still refuse at execution — general case), **P19** (backslash separators in the proposed-layout check), **P27** (empty directories left by archival), **P28** (worktree vantage point). None is believed to be a criterion violation, but say so explicitly rather than skipping them — an earlier epic failed its holistic review on exactly this step.
- [ ] Epic gates are `repo-validate` + `holistic-review`, then the completion report per `completion-report-template.md`, then archive it and link it back.

## Traps — do not repeat these

All prior traps remain in force. New or newly sharpened:

- **Do NOT filter the dispatch script's output.** I piped `dispatch-worker-worktree.sh` through `grep '^\[ok\]'`, which hid its refusal — it declines to branch from a dirty tree, and a concurrently running gate batch writes `.jit/gate-runs/` records that make main dirty. The worker sat blocked for an hour on a worktree that did not exist. Read the script's full output, and do not dispatch while a gate batch is writing.

- **Do NOT verify "this was left untouched" from the operation's own report.** Verifying that a *retained* artifact was left alone took three wrong oracles: `os.path.exists` failed on an embedded link target that never existed (`dev/reference/configuration.md`, correctly retained as a no-op and correctly non-blocking, because only an *explicitly* linked source raises `MissingSource`); `content_identity` failed because the planner records one only for an artifact it will *write*, so it is null both for a present out-of-root file and an absent one. The right oracle is git: a sweep commits only after verification, so `HEAD` is the pre-run state and `git status --porcelain -- <path>` empty means untouched, whether or not the path existed.

- **An archival plan is order-dependent; a pre-computed action table is not evidence.** Previewing all 24 containers against a pristine tree gives 175 relocations; previewing each immediately before its own execution gives 176. Each execution relinks references and so changes which owners a later container sees. Never compare a pristine measurement against an interleaved one and read the difference as a regression — I nearly did.

- **A per-run assertion does not survive being applied to the final tree.** A retrospective pass flagged `14303b30` for a mirrored artifact absent at source. Correct per-run *and* correct at the end: `json-output-standardization-plan.md` has two owners; `14303b30` mirrors it, then `9d427a6b` relocates it. Model the sequence — index later relocations before asserting an earlier run's retention.

- **A rehearsal must end in validation, not in exit codes.** All 48 rehearsal commands exited 0 with empty stderr, which reads as a clean wave. `jit validate` inside the probe then failed immediately. Eligibility, exit status and action counts were all blind to it. Extend N11: run the project's whole-repository validation *inside the probe* and diff against the pre-run state.

- **A criterion that can only be observed once needs its observation committed.** `ca832358`'s REQ-05 had been verified twice and code-review still failed it as "materially unverified" — correctly, because no attributable evidence existed in the repository. Record it in the issue's **own** canonical directory (`jit doc dir <issue> dev/active`), not the epic's, or `jit doc conformance` reports it misplaced.

- **Quote the criterion in a dispatch prompt; do not paraphrase it tighter.** I told `8ff4cff3`'s worker REQ-05 meant byte-identity of a whole quoted TOML block. It meant the declaration. The stricter reading would have forced two long `description` strings into a teaching excerpt, which the same prompt's scope boundary forbade. The worker asked instead of guessing. Separately, my own wording directive to `334bcd6f` introduced the inaccuracy its next doc-review caught ("the suffixed preferred root" where the preferred root is bare without a membership label).

- **When a batched gate fails, check the remaining entries for a shared surface before letting the batch continue.** A stale citation failing `docs-mechanical` fails it for every issue carrying that gate. I stopped a batch twice for this; each stop cost about a minute and saved review rounds.

- **Do NOT trust a worker's path-resolution claim made from its worktree.** `git worktree add` materialises only tracked paths and git tracks no empty directory, so a worktree lacks the eight directories archival emptied on `main`. Two workers reached wrong conclusions through this in one session: one ran `docs-mechanical` in its worktree, saw `MISSING: dev/plans` on unmodified docs and reported a pre-existing defect at HEAD (the checker resolves against cwd; on `main` it passes); the other walked its worktree for empty directories and wrote "no empty directory is left behind", true of the tree it inspected and false of the repository. My own first correction of the first case was also wrong. Every remaining issue is a documentation issue dispatched into a worktree and gated on `main`, so warn each worker: a `MISSING:` line naming a `dev/` directory is this class, not their defect, and the fix is a placeholder rather than a real area name.

- **Do NOT transcribe an enumeration decided at planning time.** Six of the five disposition issues' 72 enumerated assertions were false, all on mirrored rows — 22 named mirror destinations of which 20 hold no copy, two files never planned by any run at all, two enumerated as mirrored when the run had moved them. Recording any as enumerated would have falsified a REQ-03. `@/issue/8e071e18/decision/D-20` calls these outcomes "decided at planning time and executed without judgement", which describes the *command*, not the record. Dispatch such an issue with verification as the primary instruction and name one known mismatch so the worker has the shape.

- **`jit issue create` has no `--description-file`.** Use `--description "$(cat file)"`. `jit issue update` *does* have `--description-file`.

- **`jit dep add X Y --reduce` can silently no-op** when Y is already transitively reachable. Check the resulting edge set, not the command's success — and re-add after removing the edge that made it redundant.

## Open questions needing invoker input

None. E13–E17 are all resolved and applied.

## Reference artefacts

- Epic: `jit issue show 8e071e18`
- Plan: `dev/active/8e071e18-plan.md`; manifest: `dev/active/8e071e18-breakdown.json` (79 entries; `fd88adda`, `ac45f567`, `0e5dff57`, `84c4e956`, `ca832358`, `334bcd6f` are not in it)
- Investigation: `dev/active/8e071e18-investigation.md` (its addendum supersedes earlier sections)
- Progress file: `dev/active/8e071e18-progress.json`
- **This epic's own records, all in `dev/active/8e071e18-dev-artifact-layout/`:** `archive-run-evidence.md` (per-run artifact tables and citation warnings — the only record of the plan each run executed, since re-previewing an archived container now reports a no-op), `archive-completeness-record.md`, `citation-warning-consolidation.md`
- `dev/active/ca832358/req05-archival-execution-evidence.md` — the 24-container execution probe
- Follow-ups filed outside the epic this session: `f9e42a43`, `f289ff18`
