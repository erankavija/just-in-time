# Handoff — epic 287c4051 (Documentation accuracy & audience-boundary cleanup)

**As of:** 2026-07-10, main HEAD after the round-2 gate-record commit.
**Skill:** resume with `/jit-execution-lead`. Progress state is `dev/active/287c4051-progress.json` (authoritative; read it first).
**Coordination:** a parallel execution-lead session shares `main`. One worktree = one writer; never touch its worktrees. `events.jsonl` is union-merge. Commit their in-flight `.jit` state only when the tree validates.

## Where things stand

Epic drives 5 hard success criteria:
- REQ-01 shipped doc statements match current source tree
- REQ-02 adopter docs describe shipped surface only; repo-local dogfood config relocated to `dev/` or signalled
- REQ-03 no legacy/in-flight narration
- REQ-04 a doc-review gate exists, is required on the epic, passes before completion
- REQ-05 every diagram in `docs/` is Mermaid; ASCII-art forbidden

Waves 1–3 done. Wave 4 (the doc-review gate's first 21 findings) **all merged into main** across dw-a/b/c/d and verified. Workers released and idle.

**Current blocker:** re-running `doc-review` on the merged tree (`84359f2f`) **FAILED** with **5 fresh residual findings** (the round-1 audit under-counted; these sit mostly outside wave-4 scope). The gate confirmed all 21 wave-4 fixes landed. Run record committed under `.jit/gate-runs/98965034-.../`.

## The 5 remaining findings — all confirmed real, all REQ-03/REQ-01 class

Handle as **lead edits directly on main** (same category as the wave-2 residual narration fixes; they are small, localized, source-verifiable). Source of truth already checked for each.

| # | File:line | Defect | Remediation (verified) |
|---|-----------|--------|------------------------|
| F1 | `docs/concepts/guarantees.md:352-361` | Diagram nests `.git/jit/` **under** `.jit/` and names `heartbeats/` | `.git/jit/` is a repository-root sibling of `.jit/`, not a child (`control_plane.rs:24-25` creates it at git-dir root). Subdir is singular `heartbeat/` (`control_plane.rs:32`). Mirror the authoritative layout in `storage-format.md:244-250`: show `.jit/` and `.git/jit/` as two separate root trees; subdirs `claims.jsonl`, `claims.index.json`, `heartbeat/`, `locks/claims.lock`. |
| F2 | `docs/reference/example-config.toml:102-109` | Migration history: "the former `require_type_label`, … keys **were removed**" | Delete the removed-key history. State only: label/type enforcement is defined in `.jit/rules.toml` (single source of truth); this `[validation]` section carries only the current behavioral keys above. |
| F3 | `docs/how-to/custom-gates.md:222-223` | "a checker that emits no block keeps working **exactly as before**" | State current behavior directly: a checker that emits no block retains its raw stdout and exposes no structured finding fields. |
| F4 | `docs/examples/research/rules.toml:50, 65, 195, 207, 227` | Four `in-flight` markers + one "behaves **exactly as before**" | `in-flight` is on the dimension-3 prohibited-marker list. Replace each with the precise non-`done` states (line 207 already enumerates `backlog, ready, in_progress, gated` right after it — so `in-flight` is redundant there). Line 227: state that for an unbracketed goal the exclusion is a no-op. |
| F5 | `docs/examples/sdd/rules.toml:47, 191` | "an **in-flight** epic" (x2) | Name the exact applicable lifecycle states, e.g. "an epic whose state is not `done`" / the planning states. |

**Do NOT sweep em-dashes** (LD-11): the doc-review gate does not grep punctuation — its content-standards dimension checks Mermaid/LaTeX only. The 153+82 pre-existing em-dashes flagged by dw-c/dw-d are out of contract. The em-dash ban is a lead/global style rule, not this epic's gate.

## Resume procedure

1. Fix F1–F5 (edits above). Sweep for the same markers in the two other worked configs if any (`docs/examples/*/rules.toml`, `*/config.toml`) — grep `in-flight|exactly as before|were removed|no longer|formerly` across `docs/examples/`.
2. Re-run: `jit gate evaluate 6ce0f14e doc-review --json` **from the main working directory** (gate evidences the working tree, not the issue's worktree — bug 45e1b7e8). ~30-min codex run; background it.
3. Loop until `VERDICT: PASS`. Fresh passes may surface more residue; each round is cheap relative to shipping a wrong doc.
4. On green: complete `6ce0f14e` — confirm `jit-validate`, `cargo-ci`, `code-review`, `doc-review` all pass, then transition to done. (LD-7: ensure a succeeded `cargo-ci` gate exists before code-review so the reviewer skips `cargo test` and dodges the `find_available_port` race, bug 894337e2.)
5. **Epic completion (section 10):** run epic gates (`repo-validate` + `doc-review`); map all 5 success criteria to delivering issues; write completion report from `references/completion-report-template.md`; transition epic to done; archive `287c4051-progress.json` + report and link via `jit doc add`; delete this handoff.

## Ground rules carried forward
- Worktree dispatch: `scripts/dispatch-worker-worktree.sh` (manual `git worktree add -b … <main-HEAD-SHA>`); never Agent `isolation:"worktree"` (stale-base trap). `dangerouslyDisableSandbox:true` for git writes in worktrees.
- Reviewer verdict is inviolable; comply or escalate, never soften the contract. MAX_REWORK_ATTEMPTS=2 before escalating.
- LD-1..LD-11 recorded in the progress file; LD-8 (deleted dead `storage::claims_log`), LD-10 (21 findings as wave 4), LD-11 (no em-dash sweep) are the load-bearing recent ones.
