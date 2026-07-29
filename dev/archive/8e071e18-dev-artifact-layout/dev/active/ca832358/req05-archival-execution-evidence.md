# REQ-05 evidence — whole-repository validation after every terminal container plan

Issue `ca832358`. This record exists because the criterion is an end-to-end
property of *this* repository rather than of a fixture, and it can only be
observed by executing archival for real. Executing it on `main` would consume the
one observation the cleanup issues are there to make, so the verification runs in
a throwaway detached worktree and the worktree is discarded afterwards. What is
committed here is the run's own output.

## Reproduction

```bash
git worktree add --detach /tmp/req05-probe HEAD
cd /tmp/req05-probe
for c in 14303b30 2821e177 53e3fa36 2e926e39 4a00b2b0 7095769d 6eb585bc 71373e37 94f873c8 cfb3ba94 a4e3cfb0 2d109173 9ac9fdac 90a2dbfd d7bfd4a4 ad601a15 2fbd2a82 9d427a6b 5fe00921 f2532a2d 93f3e4df 9b7b5f9c 1cc809de 25064508; do
  jit archive container "$c" --json         > "$c.preview.json" 2> "$c.preview.err"
  jit archive container "$c" --execute --json > "$c.exec.json"  2> "$c.exec.err"
done
jit validate --json
cd - && git worktree remove --force /tmp/req05-probe
```

Each container writes to its own file: reusing one shell variable inside the loop
reads the previous iteration's value and misattributes every message.

## Run

Probe worktree at `9d48ee13`, one detached checkout of `main`. All 24
containers previewed and executed in the order above.

| container | destination root | relocated | mirrored | retained | already archived | retained carrying a reference change | execute stderr bytes |
|---|---|---|---|---|---|---|---|
| `14303b30` | `dev/archive/14303b30-phase5-2` | 8 | 1 | 4 | 3 | 0 | 0 |
| `2821e177` | `dev/archive/2821e177-addressing-v2` | 11 | 0 | 5 | 2 | 0 | 0 |
| `53e3fa36` | `dev/archive/53e3fa36-agent-ergonomics` | 2 | 0 | 1 | 1 | 0 | 0 |
| `2e926e39` | `dev/archive/2e926e39-agent-seamlessness` | 5 | 0 | 1 | 0 | 0 | 0 |
| `4a00b2b0` | `dev/archive/4a00b2b0-agent-validation` | 3 | 0 | 0 | 0 | 0 | 0 |
| `7095769d` | `dev/archive/7095769d-code-smell-cleanup` | 1 | 0 | 0 | 0 | 0 | 0 |
| `6eb585bc` | `dev/archive/6eb585bc-core-maintenance` | 17 | 7 | 10 | 6 | 0 | 0 |
| `71373e37` | `dev/archive/71373e37-docs-lifecycle` | 6 | 2 | 1 | 1 | 0 | 0 |
| `94f873c8` | `dev/archive/94f873c8-docs-lifecycle-p2` | 3 | 0 | 0 | 0 | 0 | 0 |
| `cfb3ba94` | `dev/archive/cfb3ba94-docs` | 1 | 1 | 2 | 1 | 0 | 0 |
| `a4e3cfb0` | `dev/archive/a4e3cfb0` | 1 | 0 | 0 | 0 | 0 | 0 |
| `2d109173` | `dev/archive/2d109173-docs-exhaustive-audit` | 16 | 0 | 10 | 6 | 0 | 0 |
| `9ac9fdac` | `dev/archive/9ac9fdac-graph-templates` | 6 | 0 | 2 | 1 | 0 | 0 |
| `90a2dbfd` | `dev/archive/90a2dbfd-item-sources` | 2 | 0 | 1 | 1 | 0 | 0 |
| `d7bfd4a4` | `dev/archive/d7bfd4a4-observability` | 3 | 0 | 0 | 0 | 0 | 0 |
| `ad601a15` | `dev/archive/ad601a15-parallel-work` | 15 | 0 | 0 | 0 | 0 | 0 |
| `2fbd2a82` | `dev/archive/2fbd2a82-planning-bracket` | 4 | 0 | 3 | 1 | 0 | 0 |
| `9d427a6b` | `dev/archive/9d427a6b-production-polish` | 13 | 0 | 2 | 2 | 0 | 0 |
| `5fe00921` | `dev/archive/5fe00921-production-stability` | 14 | 0 | 0 | 0 | 0 | 0 |
| `f2532a2d` | `dev/archive/f2532a2d-jit-project-lead` | 11 | 4 | 30 | 1 | 0 | 0 |
| `93f3e4df` | `dev/archive/93f3e4df-rejection-state` | 1 | 0 | 0 | 0 | 0 | 0 |
| `9b7b5f9c` | `dev/archive/9b7b5f9c-jit-profiles` | 10 | 0 | 1 | 1 | 0 | 0 |
| `1cc809de` | `dev/archive/1cc809de-repository-state-quality` | 18 | 4 | 5 | 3 | 0 | 0 |
| `25064508` | `dev/archive/25064508-structured-knowledge` | 5 | 0 | 5 | 3 | 0 | 0 |

Every preview and every execution exited 0, and every execution's stderr was
empty. No plan reported a blocker: 0 across all 24, and no plan was
ineligible (0 ineligible).

## Whole-repository validation in the probe, after all 24 executions

```json
{
  "divergence_count": 0,
  "error_count": 0,
  "integrity_error": null,
  "membership_divergences": [],
  "message": "Repository validation passed",
  "rule_findings": [],
  "valid": true,
  "warning_count": 0,
  "warnings": []
}
```

Zero errors, zero warnings, zero membership divergences, and no integrity error,
so every document reference in the repository resolves.

## The defect this replaces

The same 24 executions at `51d1977f`, before the fix, left `jit validate`
failing:

```
Invalid document reference in issue 'c8355d70': file
'dev/archive/cfb3ba94-docs/docs/concepts/design-philosophy.md' not found in the
working tree or at HEAD
```

33 retained artifacts across 8 of the 24 containers carried a reference change to
a destination their run never wrote. Every one of the 24 previews reported
`eligible: true` with zero blockers and every execution exited 0, so neither
eligibility nor exit status revealed it.

## Nothing else moved

The 24 plans were computed twice over one unchanged repository state, varying only
the binary: once with the pre-fix build and once with the post-fix build, each
invoked by explicit path. Holding the input fixed is what makes the two columns
comparable.

It has to be held fixed, because an archival plan is order-dependent: each
execution relinks references and so changes which owners a later container sees.
Previewing all 24 against a pristine tree gives 175 relocations,
while previewing each one immediately before its own execution — the order the run
table above reports — gives 176. The one artifact that differs has
two owners, and the first container in the order mirrors it and relinks the owner
that sits outside the later container's subtree, after which only the inside owner
remains and the action is a relocation rather than a mirror. Comparing a pristine
measurement against an interleaved one would read that as a regression.

| measure | pre-fix | post-fix |
|---|---|---|
| relocated / mirrored / retained | 175 / 20 / 83 | 175 / 20 / 83 |
| relocated artifacts carrying a reference change | 152 | 152 |
| mirrored artifacts carrying a reference change | 13 | 13 |
| retained artifacts carrying a reference change | 33 | 0 |

The action counts are identical and the relink counts for the two actions that
write a destination are unchanged, so the change removes exactly the relinks that
named a path nothing writes.

## A neighbouring property this does not claim

A document *reference* is a record jit holds; an in-content *link* is text inside a
document. `jit validate` resolves the former, which is what REQ-05 names.
`jit doc check-links` additionally follows the latter, and its count over the same
probe fell from 15 errors before the runs to 12
after.

11 link errors are new and 11 disappeared, and they are the same
links read from a new location: each moved with its document.

Ten sit on a document the runs *mirrored* — `dev/benchmarks/rust-build-efficiency/report.md`
and `dev/eval/lead-skills-eval-baseline.md` — whose live original is still in place
in the post-run tree and still resolves, so only the frozen archive snapshot
carries a dead link, which is what a snapshot of a moved-on repository is
(`@/issue/8e071e18/decision/D-14`).

The eleventh sits on `dev/active/4c33d0e5-audit-notes.md`, which the runs
relocated — and the pre-run tree already reported that same link broken from the
live path, so relocation carried an existing defect along rather than creating
one.

So the runs introduce no link that is dead from both sides. The oracle for the
split is the probe's post-run tree, where a mirrored document's original survives
and a relocated one's does not; measuring against `main` would find every source
present, because `main` has archived nothing.

Archival deliberately never rewrites document content, so a relocated historical
record keeps its citations verbatim and a human decides.
