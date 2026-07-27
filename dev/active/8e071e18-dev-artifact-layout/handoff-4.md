# Handoff — Issue-based development artifact layout and serviceable archival (8e071e18) — session 6

**Date:** 2026-07-27
**Session number:** 6 (session 5 produced no handoff; its work is reconstructable from `git log` and the progress file)
**Prior handoffs:** `dev/active/8e071e18-handoff.md` (1), `dev/active/8e071e18-handoff-2.md` (2), `dev/active/8e071e18-dev-artifact-layout/handoff-3.md` (4). Their Traps remain in force except where this handoff records a resolution.

## Current state

- Epic: `8e071e18` — in_progress, claimed `agent:jit-execution-lead`
- Wave 9 of 13. **Waves 1–8 complete**, including all 24 archival executions.
- Children summary: **70 done**, 12 in_progress, 6 backlog/ready, 0 rejected (88 total incl. the epic)
- Active claims: the 12 in_progress are `58c56743`, `aa5db9a1`, the five dispositions (`5a19fffb`, `66c467c1`, `93aaa2b2`, `2d7ae27e`, `079ea42e`), the five verification-only citation issues (`4790367e`, `c7e3ac1b`, `6e963a7f`, `560c2adb`, `d52bd541`) — plus `0b3840f1`, whose two reviews failed and which re-gates last. **`jit issue list --assignee agent:worker` is useless for this** — it returns ~135 issues, most of them long-closed.
- Open escalations: **none open.** E13–E17 were all raised and resolved this session.
- Progress file: `dev/active/8e071e18-progress.json` (waves, per-issue status, E1–E17, P1–P26, N1–N31)

**Three workers were in flight when this was written.** Check them before anything else: `w-58c56743`, `w-aa5db9a1`, `w-dispositions` (worktree `agent-5a19fffb`, covers all five disposition issues).

## What just happened

- **The 24 archival executions ran and are all closed.** Sequential on main, one commit pair per issue, each verified before its commit and gated with `repo-validate`. Totals: 176 relocated, 19 mirrored, 83 retained, 0 blockers, 24 recorded execution events. Then re-verified under one uniform assertion set including byte-for-byte hash comparison of 214 files against the plan that moved them. `jit validate` clean. Evidence: `archive-run-evidence.md` (linked to the epic).
- **`ca832358` filed and landed — a criterion blocker the rehearsal caught.** A retained artifact was still relinked to a destination the run never writes: 33 across 8 containers, making `jit validate` fail after the runs. Root cause: `select_direct_owners` set `selected_for_relink` before the action was chosen, and `reference_changes` never tested the action. Fixed by re-gating both on the finalized action. A/B over one unchanged state: actions identical, retained-with-reference-change 33 → 0.
- `84c4e956` closed: two out-of-root citations rewritten root-relative; asset scan refreshed.
- `334bcd6f` filed and closed (E14): three false/absent facts in `docs/reference/cli-commands.md` that blocked `89b5899a`'s doc-review.
- `89b5899a`, `35499d1e`, `8ff4cff3`, `aa38b236` closed. `aa38b236` produced the completeness record (138 files remain under the managed areas, per-reason 9/3/4/122) and the consolidated warning list (641 occurrences, 105 citing files).
- `623e4c46` and `834a78a8` **pulled forward** and closed — 7 stale citations that were failing `docs-mechanical` for the whole epic.
- Five citation issues verified as needing **no edit** and gated: their cited artifacts were mirrored, not relocated.
- Owner decisions: **E13** (two criteria named the pre-E9 copy mechanism → amended to retention), **E15** (epic REQ-09 scoped to terminal issues outside the epic's own subtree), **E16** (two product bugs filed: `f9e42a43`, `f289ff18`), **E17** (reorder: documentation before checkpoints).
- Confirmed the previously-unverified half of N8: the stale-binary guard tolerates a moved HEAD when no build input changed.

## What to do next

- [ ] **Check the three in-flight workers first** (`w-58c56743`, `w-aa5db9a1`, `w-dispositions`). Merge, gate, close each. `aa5db9a1`'s worktree is `agent-aa5db9a1` at base `1d9d7f43`; the dispositions worktree is `agent-5a19fffb` at base `2741cc38`.
- [ ] Close the five verification-only citation issues once their gate batch finishes: `4790367e` (cargo-ci + code-review), `c7e3ac1b` (code-review), `6e963a7f`/`560c2adb`/`d52bd541` (doc-review + docs-mechanical). Their verification is recorded in their claim commits — read those before writing a verdict.
- [ ] Then `4a6fb8c1` (needs `58c56743` done). It owns `docs/reference/cli-commands.md` outright and must add `jit doc dir` and `jit doc conformance` entries — that is the finding two reviewers raised on `0b3840f1`. Its REQ-02 was amended under E13; read its Notes.
- [ ] Then `5e9305ec` (needs `58c56743`).
- [ ] **Re-run `0b3840f1`'s doc-review and code-review last**, after `4a6fb8c1` lands. Both failed on the same two findings; nothing else on it is outstanding (repo-validate, docs-mechanical, mcp-ci, cargo-ci all passed).
- [ ] Then `143388dc` (5 gates, needs all 13 cleanup issues) and `af264b06` (needs `4a6fb8c1` + `5e9305ec`).
- [ ] Before the epic's own gates, reconcile `surfaced_pitfalls` P1–P26 against the 15 `[hard]` criteria per `lead-review-protocol.md`. Still needing a disposition call: **P18** (an eligible plan can still refuse at execution — general case, not just fragments), **P19** (backslash separators in the proposed-layout check), **P23** (closed this session), **P26** (adjudicated as a non-defect).
- [ ] Epic gates are `repo-validate` + `holistic-review`. Then the completion report.

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
