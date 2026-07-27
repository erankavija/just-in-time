# Epic Complete: Issue-based development artifact layout and serviceable archival (8e071e18)

**Started:** 2026-07-25
**Completed:** 2026-07-27
**Assignee:** agent:jit-execution-lead

## Summary

Repository-local artifact layout is now an issue-based directory convention that the archival
mechanism can service, and the development root has been cleaned up by running that mechanism rather
than by hand. `jit archive` classifies every configured development area, retains what lies outside
the development root, treats a directory link target as navigation, and warns about in-content
citations it would break. `jit doc dir` resolves an issue's canonical directory and `jit doc
conformance` advises on artifacts that sit outside one.

## Metrics

| Metric | Value |
|---|---|
| Children completed | 89 / 89 (0 rejected) |
| Waves executed | 14 |
| Rework cycles | 8 across 7 issues |
| Escalations | 19 |
| Issues created during execution | 8 |
| Sessions | 7 |
| Findings the lead recorded and dispositioned | 31 |

## Success criteria

| criterion | delivered by |
|---|---|
| REQ-01 fresh-repository area classification | `69dd4e36`, `b3fc1a92` |
| REQ-02 eligible plan for a presentation deck | `daf4fa46` |
| REQ-03 canonical directory shape | `22c11e40` |
| REQ-04 a command resolves the directory | `58c56743` |
| REQ-05 filenames carry no short-id prefix | `429995e1`, `8ff4cff3` |
| REQ-06 advisory conformance report | `e2902949`, `f89ea266` |
| REQ-07 archive naming with no title fallback | `b4d1a2c8` |
| REQ-08 in-content citation warnings | `3be32cad` |
| REQ-09 cleanup through `--execute` | the 24 sweeps, the 8 citation issues, `d8d7f317` |
| REQ-10 disposition for every unprefixed artifact | `079ea42e`, `2d7ae27e`, `5a19fffb`, `66c467c1`, `93aaa2b2`, `d9e70703` |
| REQ-11 agent skills obtain paths from the convention | `935b51da` |
| REQ-12 one canonical adopter home | `4a6fb8c1`, `58c56743`, `5e9305ec` |
| REQ-13 pre-existing flat artifacts stay resolvable | `aa38b236` |
| REQ-14 out-of-root artifacts retained | `0e5dff57`, `1f80212b`, `4f9af089` |
| REQ-15 directory link target is navigation | `7d6d2783` |

Verified independently of the delivering issues on a freshly initialized repository, so the evidence
does not depend on this repository's own `.jit/`: REQ-01 (the scaffolded `[documentation]` table
carries eight managed paths, six permanent entries and four issue-scoped areas with no hand editing),
REQ-02 (`jit archive document` and `jit archive container` both eligible for a deck owned by a
terminal issue, zero blockers), REQ-07 (a container with no membership label takes the bare short-id
destination, not its title), REQ-08 (`moving-path-citation` warnings with 1-based line and column for
a backtick span and a shell comment, **identical with and without version control**, honouring
`@/charter/D-4`), REQ-14 (source, script and repository-root artifacts all `retain`, no
`unmanaged-selected-root` blocker) and REQ-15 (a directory link contributes no artifact, no edge and
no warning).

## What the epic's own gate caught

The `holistic-review` gate failed twice, and both findings were real.

**REQ-09.** One artifact remained: `dev/active/fdb039ee-modular-document-rendering.md`, owned by a
terminal story of the terminal container `41d07c6b`, outside this epic's subtree. The container was
never swept because its whole archivable content was that one artifact and the planner retained it —
its document reference carried a commit pin written by `fe7667ed` when that issue structure was
created, with no recorded purpose and no script reading the path. REQ-09 states no commit-pin
exception. Escalated (E19); the owner chose to unpin and execute rather than amend the criterion.
`d8d7f317` did so: the artifact is now at
`dev/archive/41d07c6b-usability/dev/active/fdb039ee-modular-document-rendering.md` with an identical
blob, its reference relinked, and the population REQ-09 ranges over re-derived to zero.

**REQ-06.** `jit doc conformance` resolved an owner only from a filename's short-id prefix and passed
over names carrying none. The lead had recorded this as pitfall P29 and argued it was *not* a
violation, reasoning from `@/issue/8e071e18/decision/D-2` that location names the owner. The review
disagreed and was right, for a sharper reason: D-2 strips the prefix from filenames inside an issue
directory, so every artifact created under the convention is prefix-less by construction and the
report goes blind exactly as the convention is adopted. `f89ea266` closed it additively — the
short-id rule still decides first, and a document reference resolves an owner only where it declines.
The report went from 44 artifacts to 54 with none removed and none changed, measured two ways.

The lead's reconciliation of all 31 recorded findings against the fifteen `[hard]` criteria is at
`pitfall-criterion-reconciliation.md`. That it argued one entry wrongly is the point worth carrying:
writing the reasoning down did not pre-empt the reviewer, but it made the disagreement settleable in
one round.

## Open findings for an owner decision

None blocks the epic; none is a criterion violation.

| id | finding |
|---|---|
| P18 | an eligible plan can still refuse at execution in the general case; the fragment-bearing instance was closed by `ac45f567`, the design question is open |
| P19 | the proposed-layout check and discovery diverge on backslash separators; latent, and the durable fix is one shared normalizer rather than a second hand-aligned `replace` |
| P25 | a directory link target is navigation to archival discovery and a missing asset to `jit doc check-links`; the largest of these and the most likely to reach an adopter |
| P27 | archival leaves empty source directories, invisible under version control and persistent without it |
| P29 | the conformance report still never consults a document reference to *contradict* a filename prefix, only to supply a missing one |
| P30 | `dev/eval/lead-skills-eval-baseline.md:18-19` cites two `evals/results.md` files that do not exist; pre-existing and outside the epic |
| P31 | seven of the epic's own evidence records sit in the epic's directory while their references name the child issues that wrote them, so the repository's own report names them |

P31 is the one worth a decision. Both repairs cost something the epic already ruled against:
re-pointing the references to `8e071e18` destroys the child-to-artifact traceability
`@/issue/8e071e18/decision/D-14` refused to destroy, and relocating the files means repointing
citations inside dated evidence records that `@/issue/8e071e18/decision/D-8` requires to survive
verbatim. Left as-is deliberately, which `D-4` and `D-7` between them make the designed case.

## Escalations

Nineteen, all resolved. E19 is described above. E13 (two criteria still named the pre-E9 copy
mechanism), E15 (REQ-09 scoped to terminal issues outside the epic's own subtree), E16 (`f9e42a43`
and `f289ff18` filed) and E17 (documentation reordered ahead of the checkpoints after a finding
recurred past `MAX_SAME_FINDING_REPEATS`) were the other decisions that changed the epic's shape.

## Issues created during execution

`fd88adda`, `ac45f567`, `0e5dff57`, `84c4e956`, `ca832358`, `334bcd6f` from execution findings, and
`d8d7f317`, `f89ea266` from the epic's own gate.

## Holistic quality notes

- **The central repair was exercised on real data, not demonstrated.** Twenty-four container
  archival executions relocated 176 artifacts, mirrored 19 and retained 83 with zero blockers, then
  were re-verified under one uniform assertion set including byte-for-byte comparison of 214 files
  against the plan that moved them. `ca832358` was filed from that rehearsal and fixed a real defect
  it exposed.
- **Two issues can each satisfy their criteria and together drop a fact.** `4a6fb8c1` was told to
  remove the slug derivation and cite the canonical home; `58c56743` was told to state how the name
  is derived. Both complied, and the 48-character bound and collapsing rules briefly existed nowhere
  on the adopter surface. Each reviewer sees one file; only the lead sees the pair.
- **Enumerations decided at planning time were wrong six times out of 72.** Every one was on a
  mirrored row. Verification, not transcription, is what a disposition issue needs.
- **A lead-authored evidence record is gate evidence like any other.** The remaining-artifact
  addendum failed `code-review` for counting the tree before its own commit, and for calling its
  owner `in_progress` when the tracked state was `ready`. Both were correct.
