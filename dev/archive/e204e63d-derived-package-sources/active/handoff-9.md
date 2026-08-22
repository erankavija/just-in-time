# Handoff — Derived profile assets and projected policy documentation (e204e63d) — session 10

**Date:** 2026-08-04
**Session number:** 10
**Prior handoffs:** `handoff.md` (1) through `handoff-8.md` (9), same directory.

## Current state

**The epic is complete.** `e204e63d` is `done`. 80 issues under the epic label, all `done`.
Both epic gates passed: `repo-validate`, and `holistic-review` with one low advisory.
`jit validate` is clean. No worktrees remain except `agent-a122b9b3` and
`steward-v1-readiness`, which are unmerged and belong to other work.

Artifacts: `e204e63d-completion-report.md` and `dev/presentations/e204e63d/talk.html`, both
linked to the epic.

## What this session did

- Closed **wave 7** (`26f503cc`, `62ef09b6`, `d198030e`) — one rework round, on `d198030e`.
- Opened and closed **two remediation tasks** from blocking findings at the `7af6eb3d`
  container gate: `cd9a17f0` (a documented init command that failed when run) and `e522a8e1`
  (hierarchy type names surviving in the bridge's shipped instructions). Both passed every
  gate with no rework.
- Closed all three **container checkpoints**: `7af6eb3d`, `cc75b4e6`, `6df7e456`.
- Opened and closed `65280aff`, reconciling the **unreleased changelog** against the release
  it describes — five entries described the compiled-in package as a live route and the
  removal itself had no entry.
- Fixed two `doc-review` findings lead-direct with per-issue attribution.
- Built a **20-slide showcase deck** for adopters and profile contributors, fitcheck-clean.

## The two findings the container gate caught

Both are the reason container checkpoints exist, and both are worth remembering:

1. `fdae1023` passed its own `mcp-ci` and `code-review` while leaving `epic` and `milestone`
   in `SERVER_INSTRUCTIONS` — shipped agent-facing text. Only the container's whole-surface
   review saw it. A leaf's own gates do not see what the leaf failed to finish elsewhere.
2. `docs/reference/cli-commands.md` presented `jit init --profile jit-dogfood` as "the
   preferred setup" while the paragraph directly beneath it explained why that command
   cannot work on a first application. Both reviewers found it; running it confirmed
   `Error: Profile not found: jit-dogfood`.

## Traps — do not repeat these

All prior traps remain in force. New this session:

- **A worker can replace the host-global binary while a sibling depends on it.** The
  `cd9a17f0` worker ran `./scripts/install-jit.sh` three times from its own worktree,
  installing a binary built from that worktree onto `PATH`, while another worker's
  `npm test` resolved `jit` from `PATH`. The brief's build-discipline list forbids
  `cargo-ci.sh`, `verify-commit-builds.sh` and `jit gate evaluate` but **not**
  `install-jit.sh`. Add it: installing is a host-global write, not a targeted check.
- **A reviewer's cited repair site can be the wrong one.** `code-review` F1 on `d198030e`
  diagnosed the failure correctly and pointed at a CLI `about` string that `62ef09b6` had
  already aligned a shipped page to, and which was already `done` and gated. Editing it
  would have created a falsehood in finished work. Carry the constraint *against* the
  reviewer's site into the rework brief, with the reason, rather than following it or
  silently ignoring it. The next round passed.
- **`plan_hash` is not `package_hash`.** The plan hash addresses the plan and differs
  between repositories and states; the package hash addresses the bytes and is identical
  everywhere. A deck slide printed a literal plan hash as if reproducible. Do not present
  one as a stable value.
- **`profiles/jit-dogfood` is the package SOURCE, not a package.** Copying it and applying
  fails with `declared package source 'assets/live/...' is missing`. A usable package comes
  from `scripts/assemble-package.sh <dest>`. `profiles/jit-default` has no live assets and
  is usable as-is. This cost two failed verification attempts.
- **`scripts/assemble-package.sh` has no `--help`.** It takes one positional destination, so
  `--help` assembled a 64-file tree into `./--help` inside the main checkout. Removed by hand.
- **Three agents idled repeatedly without ever delivering a report** (`worker-26f503cc`, and
  both deck reviewers), across explicit re-requests naming the required format. Their work
  was fine; the reports never came. Budget for the lead verifying independently — that is
  what happened here for every criterion and every deck slide, and it found two deck errors.
- **zsh does not word-split unquoted variables.** `for pair in "a b"; do set -- $pair` passes
  the pair as one argument; five gate evaluations no-opped with `Issue not found: cd9a17f0
  docs-mechanical`, which reads like a jit error rather than a shell error. Use a function
  taking named arguments.
- **`check-leak-into-main.sh` reports the lead's own state as a leak.** Gate-run records,
  `events.jsonl` and issue JSON written after the pre-dispatch snapshot are lead state. Read
  the reported entries before acting.
- **A chained `jit gate evaluate` sequence can stop silently on a stale claims lock.** Two
  gates recorded nothing and the shell exited zero. `jit recover` cleared it. Read back
  `jit gate status-all` rather than trusting the chain's exit.

## Follow-ups, still unfiled

These have no container and placing them is the owner's call. The full list with reasoning is
in the completion report under "Follow-ups Not Filed": the `jit issue create
--description-file` gap; `jit gate` not evaluating one gate across several issues;
`verify-commit-builds.sh` possibly vacuous; `cargo-ci` non-determinism under host load; the
unguarded packaged-contribution-versus-registry drift shape (two instances); the
dogfood-guidance region's two unbound carriers; `assemble-package.sh --help`; the
stale-binary guard's same-commit-both-sides message; and a `(REQ-11)` tag naming another
epic's requirement id.

## Reference artefacts

- Completion report: `e204e63d-completion-report.md` — criteria-to-issue mapping with the
  evidence for each, metrics, escalations, and the holistic advisory.
- Deck: `dev/presentations/e204e63d/talk.html` — opens in any browser; `S` for notes.
  Re-run `python3 ~/.claude/skills/presentation-toolkit/scripts/fitcheck.py` after any edit.
- Progress file: `progress.json`, 47 `surfaced_pitfalls`.
- Session tooling in the scratchpad, not committed: `merge-and-gate.sh`, `dispatch-codex.sh`,
  `dispatch-wave.sh`, `mkbrief.py`, `briefs/`, `logs/`. Copy forward if another container
  reuses this shape.
