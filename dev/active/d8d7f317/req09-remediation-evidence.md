# REQ-09 remediation — the usability container's modular-renderer artifact

Evidence for issue `d8d7f317`, criteria REQ-01 through REQ-04, and the record that supersedes three
rows in the archival completeness record.

## What the epic's holistic review found

The review read epic criterion REQ-09 literally — every `dev/active` artifact owned by a terminal
issue outside this epic's own subtree is archived through `jit archive container --execute` — and
found `dev/active/fdb039ee-modular-document-rendering.md` still live. Its owner `fdb039ee` is a
terminal story of `41d07c6b` (Epic: Usability Improvements), itself terminal and outside this epic's
subtree. REQ-09 states no commit-pin exception.

## Why the container was never swept

`41d07c6b`'s whole archivable content is that one artifact, and the planner retained it: the
document reference carried a commit pin, and a pinned reference is retained by design. A container
whose only artifact is retained relocates nothing, so it fell outside the set of 24 executions.

The pin had no recorded purpose. `git log -S` locates it in `fe7667ed`, the commit that created that
issue structure, rather than in any decision to hold the document at a historical version — unlike
the two `dev/studies/perf` records, which `d9e70703` pinned deliberately because
`scripts/benchmark-session-cost-selftest.sh` reads one of them by path at runtime. No committed
script reads this path.

The owner therefore directed unpinning and executing rather than amending REQ-09.

## REQ-01 — the reference carries no commit pin

`jit doc add fdb039ee dev/active/fdb039ee-modular-document-rendering.md` re-recorded the reference
with `--commit` omitted, which the command documents as storing it unpinned. `jit doc list fdb039ee`
then reported it as `[HEAD]` rather than pinned.

## REQ-02 — archived through the execution path

| step | result |
|---|---|
| preview before unpinning | eligible, one artifact, action `retain`, no destination |
| preview after unpinning | eligible, zero blockers, `destination_root` `dev/archive/41d07c6b-usability`, action `move` |
| in-content citation warnings | 0, confirmed independently by grepping the configured scan roots for the path outside `dev/archive` and this epic's own records |
| execution | `2 publication(s), 1 reference change(s), 1 source deletion(s)`, exit 0 |

Afterwards the artifact is at
`dev/archive/41d07c6b-usability/dev/active/fdb039ee-modular-document-rendering.md`, the source is
absent, the `.jit-container` marker holds `41d07c6b-24ff-4d4d-8f7f-56daba93bfd7`, the document
reference resolves to the new path, and `41d07c6b` retired to `archived`.

Content is unchanged: the destination's blob id is `2320baa6f57657fde2cb879f415a93955f3ec690`, equal
to the pre-run source's blob at the parent commit.

## REQ-03 — the population REQ-09 ranges over is empty

Re-derived after the run by walking every record in `.jit/issues/`, keeping terminal issues outside
this epic's subtree, and testing each document reference for a live path under `dev/active`: **0
artifacts**. Before the run the same walk returned exactly one, this artifact. The two remaining
terminal-owner references under a managed area name `dev/studies/perf/session-cost-27ffbd2d.json`
and `dev/studies/perf/session-cost-c488ef85.json`, which REQ-09 does not range over because it names
`dev/active`, and which `d9e70703` pinned deliberately.

## REQ-04 — validation

`jit validate` passes with every document reference in the repository resolving.

## Rows this record supersedes

| record | row | superseded by |
|---|---|---|
| `archive-completeness-record.md:57` | `dev/active/fdb039ee-modular-document-rendering.md` — commit-pinned reference | archived; no longer under a managed area |
| `archive-completeness-record.md:127` | the same path named as the one file matching two reasons | the file matches neither reason now |
| `archive-completeness-record.md:150` | the `dev/active` enumeration row for that path | removed from the population |
| `remaining-artifacts-addendum.md` | the population total of 145 | see below |

The managed areas now hold 145 files: the addendum's 145, plus
`pitfall-criterion-reconciliation.md` and this record and its directory's entry written after it,
less this artifact. The completeness record's per-reason counts for `dev/active` lose one
`commit-pinned reference`, leaving the three `owner outside every archived subtree` rows and the
`live owner` rows unchanged.
