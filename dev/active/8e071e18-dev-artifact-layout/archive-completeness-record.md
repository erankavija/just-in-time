# Archival completeness record — development root

One judgement over the whole development root after the 24 container archival executions, and the
disposition of every file that remains under a managed development area. Issue `aa38b236`, criteria
REQ-01 through REQ-05; the consolidated in-content citation warning list (REQ-06) is a separate
record at `citation-warning-consolidation.md`.

**Snapshot.** The commit that adds this record, whose parent `c8cd53c2` carries the 24 archival
commits and the re-verification of all of them.

## Method

| question | how it was answered |
|---|---|
| managed development areas, archive root | `[documentation].managed_paths` and `archive_root` in `.jit/config.toml` |
| what each run planned and did | the per-run tables in `archive-run-evidence.md`, parsed row by row |
| what remains under a managed area | `git ls-files` over each managed area, plus a working-tree walk for untracked files and empty directories |
| who owns an artifact, and in what state | every `documents` entry of every record in `.jit/issues/`, indexed path → (issue, state, commit pin) |
| whether an owner sits inside an archived container | transitive dependency closure of each of the 24 container ids over the issue graph |
| whether relocated bytes changed | git blob identity of each destination at `HEAD` against the same path's source blob in the parent of the commit that wrote that container's marker |

## REQ-01 — every artifact the runs took reached the archive root

The 24 runs planned a destination for 195 artifact rows. Every destination is under the archive
root, holds the pre-run bytes, and no relocated source survives in a managed area.

| population | rows | check | result |
|---|---|---|---|
| relocated (`move`) | 176 | destination present at `HEAD`, blob identical to the pre-run source | 176/176 |
| relocated (`move`) | 176 | source absent from every managed area | 176/176 |
| mirrored (`copy`), 18 distinct sources | 19 | destination present at `HEAD`, blob identical to the pre-run source | 19/19 |
| mirrored (`copy`), 18 distinct sources | 19 | source still present, blob unchanged | 18/19 — 1 relocated by a later run (REQ-02) |

The managed areas close arithmetically over the run sequence. The parent of the first archival
commit held 311 files under the eight managed areas; the runs removed exactly the 176 relocated
sources and nothing else, and added one file (`archive-run-evidence.md`, written as the runs
proceeded), leaving 136 tracked now. This record and the consolidated warning list bring that to
138, the population REQ-03 enumerates.

Recomputing the action counts from the evidence record's per-run tables gives 176 relocated, 19
mirrored and 83 retained (33 of the retained rows already under the archive root), matching the
totals its own re-verification states. Its 214 hash-compared files are the 176 relocated
destinations plus both ends of every mirror (19 destinations and 19 sources).

### Terminal-owner references that still name a managed area

REQ-01 excepts an artifact whose reason for remaining REQ-03 records. Seven paths under a managed
area are still named by a terminal issue's document reference, and each carries such a reason — one
of the outcomes the issue names — here and in the REQ-03 enumeration. None of them is a missed run.

| path | reason it is not under the archive root | evidence |
|---|---|---|
| `dev/active/8e071e18-breakdown.json` | owner outside every archived subtree | referenced by `55fe8ec2` (done), inside no archived container's hierarchy |
| `dev/active/8e071e18-investigation.md` | owner outside every archived subtree | referenced by `55fe8ec2` (done), inside no archived container's hierarchy |
| `dev/active/8e071e18-plan.md` | owner outside every archived subtree | referenced by `55fe8ec2` (done), inside no archived container's hierarchy |
| `dev/active/ca832358/req05-archival-execution-evidence.md` | owner outside every archived subtree | referenced by `ca832358` (done), inside no archived container's hierarchy |
| `dev/active/fdb039ee-modular-document-rendering.md` | commit-pinned reference | `fdb039ee` pinned at `7ac09a40`; owner outside every archived subtree, so no run planned it |
| `dev/studies/perf/session-cost-27ffbd2d.json` | commit-pinned reference | `73981310` pinned at `b38c5b94`; owner inside `1cc809de`, retained as pinned-historical |
| `dev/studies/perf/session-cost-c488ef85.json` | commit-pinned reference | `a4b0fadf` pinned at `4ead5f12`; owner inside `1cc809de`, retained as pinned-historical |

The four `owner outside every archived subtree` rows belong to two terminal children of epic
`8e071e18`, which is `in_progress`: no run selected them because their owners lie inside no archived
container's hierarchy. Their scope is structural rather than a fixed list — every terminal issue in
this epic's own subtree owns artifacts the remaining waves still consume, so the set grows as
children close. Archiving `8e071e18` itself is what relocates them.

## REQ-02 — mirrored artifacts and their retained sources

A mirror writes the destination and leaves the source in place. Every mirror row, with the source's
status now:

| run | retained source | destination | owners at plan time | source now |
|---|---|---|---|---|
| `6eb585bc` | `dev/active/73482aa1-rust-build-efficiency.md` | `dev/archive/6eb585bc-core-maintenance/dev/active/73482aa1-rust-build-efficiency.md` | 73482aa1 | in place, bytes unchanged |
| `71373e37` | `dev/active/documentation-lifecycle-design.md` | `dev/archive/71373e37-docs-lifecycle/dev/active/documentation-lifecycle-design.md` | 71373e37 | in place, bytes unchanged |
| `14303b30` | `dev/active/json-output-standardization-plan.md` | `dev/archive/14303b30-phase5-2/dev/active/json-output-standardization-plan.md` | 0db719b1 32f804f1(outside) | relocated by a later run to `dev/archive/9d427a6b-production-polish/dev/active/json-output-standardization-plan.md` |
| `1cc809de` | `dev/architecture/repository-state-materialization.md` | `dev/archive/1cc809de-repository-state-quality/dev/architecture/repository-state-materialization.md` | 39e1c091 | in place, bytes unchanged |
| `1cc809de` | `dev/archive/features/cdc840ad/showcase/base.css` | `dev/archive/1cc809de-repository-state-quality/dev/archive/features/cdc840ad/showcase/base.css` | no owning reference | in place, bytes unchanged |
| `1cc809de` | `dev/archive/features/cdc840ad/showcase/talk.html` | `dev/archive/1cc809de-repository-state-quality/dev/archive/features/cdc840ad/showcase/talk.html` | no owning reference | in place, bytes unchanged |
| `1cc809de` | `dev/archive/features/cdc840ad/showcase/themes/rust.css` | `dev/archive/1cc809de-repository-state-quality/dev/archive/features/cdc840ad/showcase/themes/rust.css` | no owning reference | in place, bytes unchanged |
| `6eb585bc` | `dev/benchmarks/rust-build-efficiency/baseline.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/baseline.json` | no owning reference | in place, bytes unchanged |
| `6eb585bc` | `dev/benchmarks/rust-build-efficiency/optimized.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/optimized.json` | 26f97dc2 | in place, bytes unchanged |
| `6eb585bc` | `dev/benchmarks/rust-build-efficiency/post-change-test-inventory.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/post-change-test-inventory.json` | no owning reference | in place, bytes unchanged |
| `6eb585bc` | `dev/benchmarks/rust-build-efficiency/pre-change-test-inventory.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/pre-change-test-inventory.json` | no owning reference | in place, bytes unchanged |
| `6eb585bc` | `dev/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/raw/clean-1/executable-remeasure.json` | no owning reference | in place, bytes unchanged |
| `6eb585bc` | `dev/benchmarks/rust-build-efficiency/report.md` | `dev/archive/6eb585bc-core-maintenance/dev/benchmarks/rust-build-efficiency/report.md` | 26f97dc2 4e22a20d | in place, bytes unchanged |
| `f2532a2d` | `dev/eval/lead-skills-eval-baseline.md` | `dev/archive/f2532a2d-jit-project-lead/dev/eval/lead-skills-eval-baseline.md` | c23dfe71 | in place, bytes unchanged |
| `f2532a2d` | `dev/eval/skill-eval-adjudication.md` | `dev/archive/f2532a2d-jit-project-lead/dev/eval/skill-eval-adjudication.md` | a5b04c9f | in place, bytes unchanged |
| `f2532a2d` | `dev/eval/skill-triggers/README.md` | `dev/archive/f2532a2d-jit-project-lead/dev/eval/skill-triggers/README.md` | 6662f738 | in place, bytes unchanged |
| `f2532a2d` | `dev/eval/skill-triggers/run_trigger_eval.py` | `dev/archive/f2532a2d-jit-project-lead/dev/eval/skill-triggers/run_trigger_eval.py` | 6662f738 | in place, bytes unchanged |
| `71373e37` | `dev/studies/documentation-organization-strategy.md` | `dev/archive/71373e37-docs-lifecycle/dev/studies/documentation-organization-strategy.md` | 165cf162 cfb3ba94(outside) | in place, bytes unchanged |
| `cfb3ba94` | `dev/studies/documentation-organization-strategy.md` | `dev/archive/cfb3ba94-docs/dev/studies/documentation-organization-strategy.md` | cfb3ba94 | in place, bytes unchanged |

Outside the managed areas sit 8 of the sources — `dev/eval` and `dev/architecture` are permanent
paths and `dev/archive/features/cdc840ad/showcase/**` is already under the archive root — so a
mirror is the only way their content reaches a container directory. The remaining 10 are under
managed areas, and 9 of them appear in the REQ-03 enumeration; the one a later run relocated does
not.

## REQ-03 — every file remaining under a managed area, and why

The eight managed areas hold 138 files. Four of them hold none: those directories are absent from
the working tree. Every remaining file is tracked, and no empty directory is left behind.

| area | files | live owner | commit-pinned reference | owner outside every archived subtree | no document reference |
|---|---|---|---|---|---|
| `dev/active` | 49 | 9 | 1 | 4 | 35 |
| `dev/studies` | 9 | 0 | 2 | 0 | 7 |
| `dev/sessions` | 0 | 0 | 0 | 0 | 0 |
| `dev/plans` | 0 | 0 | 0 | 0 | 0 |
| `dev/presentations` | 3 | 0 | 0 | 0 | 3 |
| `dev/design` | 0 | 0 | 0 | 0 | 0 |
| `dev/benchmarks` | 77 | 0 | 0 | 0 | 77 |
| `dev/experiments` | 0 | 0 | 0 | 0 | 0 |
| **total** | **138** | **9** | **3** | **4** | **122** |

Reasons are assigned in one order, so each file carries exactly one: a live owner outranks a commit
pin, which outranks the owner's position in the graph, and a file that no reference names can only
be the last. One file matches two reasons — `dev/active/fdb039ee-modular-document-rendering.md` is
commit-pinned and its owner sits outside every archived subtree — and the pin binds, because a
pinned reference is retained by design wherever its owner sits.

Within `no document reference`, 9 files are mirror sources whose owning reference the run repointed
to the archived copy, so nothing names the source now; their evidence column says so, and they are
the one population under that reason a run did select. The other 113 were never selected: the
mechanism owns artifacts by document reference, and an issue-shaped filename prefix is not a
reference.

### `dev/active`

| path | reason | evidence |
|---|---|---|
| `8e071e18-dev-artifact-layout/archive-completeness-record.md` | live owner | referenced by `aa38b236` (this issue) |
| `8e071e18-dev-artifact-layout/archive-run-evidence.md` | live owner | referenced by `8e071e18` (in_progress) |
| `8e071e18-dev-artifact-layout/citation-warning-consolidation.md` | live owner | referenced by `aa38b236` (this issue) |
| `8e071e18-dev-artifact-layout/handoff-3.md` | live owner | referenced by `8e071e18` (in_progress) |
| `9db27a3a-progress.json` | live owner | referenced by `9db27a3a` (backlog) |
| `c639cfb5-investigation.md` | live owner | referenced by `33f76b11` (ready) |
| `c639cfb5-plan.md` | live owner | referenced by `33f76b11` (ready) |
| `c639cfb5-research.md` | live owner | referenced by `33f76b11` (ready) |
| `v1-production-readiness-scope-brief.md` | live owner | referenced by `8b05a612` (backlog) |
| `fdb039ee-modular-document-rendering.md` | commit-pinned reference | `fdb039ee` pinned at `7ac09a40`; owner outside every archived subtree, so no run planned it |
| `8e071e18-breakdown.json` | owner outside every archived subtree | referenced by `55fe8ec2` (done), inside no archived container's hierarchy |
| `8e071e18-investigation.md` | owner outside every archived subtree | referenced by `55fe8ec2` (done), inside no archived container's hierarchy |
| `8e071e18-plan.md` | owner outside every archived subtree | referenced by `55fe8ec2` (done), inside no archived container's hierarchy |
| `ca832358/req05-archival-execution-evidence.md` | owner outside every archived subtree | referenced by `ca832358` (done), inside no archived container's hierarchy |
| `13c69884-dfe9-4a62-b2ef-59fa4d2f76f3-plan.md` | no document reference | — |
| `14303b30-75b2-4b21-8963-bc6563b95b91-plan.md` | no document reference | — |
| `2fbd2a82-14ba-4e6e-90f6-e0c34f0f912c-plan.md` | no document reference | — |
| `41d07c6b-24ff-4d4d-8f7f-56daba93bfd7-plan.md` | no document reference | — |
| `49adf23b-inc6-conformance.md` | no document reference | — |
| `4a00b2b0-a527-4f04-9475-62f1d838f903-plan.md` | no document reference | — |
| `5fe00921-6548-4263-b428-f1e839f68c74-plan.md` | no document reference | — |
| `6eb585bc-998d-4778-906d-415fcdc1916e-plan.md` | no document reference | — |
| `7095769d-progress.json` | no document reference | — |
| `71373e37-bb30-41d2-af0c-f08f381e027e-plan.md` | no document reference | — |
| `73482aa1-progress.json` | no document reference | — |
| `73482aa1-rust-build-efficiency.md` | no document reference | mirror source retained by `6eb585bc`; the owning reference names the archived copy |
| `8e071e18-handoff-2.md` | no document reference | — |
| `8e071e18-handoff.md` | no document reference | — |
| `8e071e18-progress.json` | no document reference | — |
| `93f3e4df-990c-4993-b8dc-66a8827acba7-plan.md` | no document reference | — |
| `94d26c42-3f4f-4a5e-99cc-0e82475a7635-plan.md` | no document reference | — |
| `94f873c8-8508-44b0-b036-ff61c2a9716d-plan.md` | no document reference | — |
| `9ac9fdac-graph-templates-showcase/themes/gruvbox.css` | no document reference | — |
| `9b7b5f9c-handoff-2.md` | no document reference | — |
| `9b7b5f9c-handoff-3.md` | no document reference | — |
| `9b7b5f9c-handoff-4.md` | no document reference | — |
| `9b7b5f9c-handoff-5.md` | no document reference | — |
| `9b7b5f9c-handoff.md` | no document reference | — |
| `9b7b5f9c-jit-profiles-planning-brief.md` | no document reference | — |
| `9d427a6b-a8f7-4478-a0e1-1637781e00f6-plan.md` | no document reference | — |
| `abfd6016-progress.json` | no document reference | — |
| `ad601a15-5217-439f-9b3c-94a52c06c18b-plan.md` | no document reference | — |
| `cfb3ba94-7406-4bc3-9f36-cecdcebacf0e-plan.md` | no document reference | — |
| `d7bfd4a4-b2ad-477a-977e-e3242b77b7a2-plan.md` | no document reference | — |
| `documentation-lifecycle-design.md` | no document reference | mirror source retained by `71373e37`; the owning reference names the archived copy |
| `f2532a2d-handoff-2.md` | no document reference | — |
| `f2532a2d-handoff.md` | no document reference | — |
| `f6a704d0-1f01-447b-a8aa-42251fa7c3eb-plan.md` | no document reference | — |
| `planning-bracket-showcase/themes/gruvbox.css` | no document reference | — |

### `dev/studies`

| path | reason | evidence |
|---|---|---|
| `perf/session-cost-27ffbd2d.json` | commit-pinned reference | `73981310` pinned at `b38c5b94`; owner inside `1cc809de`, retained as pinned-historical |
| `perf/session-cost-c488ef85.json` | commit-pinned reference | `a4b0fadf` pinned at `4ead5f12`; owner inside `1cc809de`, retained as pinned-historical |
| `architecture-pitfalls.md` | no document reference | — |
| `clippy-suppressions.md` | no document reference | — |
| `documentation-organization-strategy.md` | no document reference | mirror source retained by `71373e37`, `cfb3ba94`; the owning reference names the archived copy |
| `jit-vs-agentic-trends-2026.md` | no document reference | — |
| `multi-agent-parallelism-analysis.md` | no document reference | — |
| `perf/session-cost-27ffbd2d.md` | no document reference | — |
| `research-workflow-examples.md` | no document reference | — |

### `dev/presentations`

| path | reason | evidence |
|---|---|---|
| `1cc809de/themes/gruvbox.css` | no document reference | — |
| `2e926e39/themes/gruvbox.css` | no document reference | — |
| `9b7b5f9c/themes/gruvbox.css` | no document reference | — |

### `dev/benchmarks`

| path | reason | evidence |
|---|---|---|
| `rust-build-efficiency/baseline.json` | no document reference | mirror source retained by `6eb585bc`; the owning reference names the archived copy |
| `rust-build-efficiency/optimized.json` | no document reference | mirror source retained by `6eb585bc`; the owning reference names the archived copy |
| `rust-build-efficiency/post-change-test-inventory.json` | no document reference | mirror source retained by `6eb585bc`; the owning reference names the archived copy |
| `rust-build-efficiency/pre-change-test-inventory.json` | no document reference | mirror source retained by `6eb585bc`; the owning reference names the archived copy |
| `rust-build-efficiency/raw/baseline-v2-clean-1/cargo-doctest-list-ignored.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-1/cargo-doctest-list.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-1/cargo-test-list.json` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-1/cargo-test-list.stderr.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-1/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-1/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-2/cargo-test-list.json` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-2/cargo-test-list.stderr.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-2/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-2/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-3/cargo-test-list.json` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-3/cargo-test-list.stderr.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-3/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-3/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-clean-samples.jsonl` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-1/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-1/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-1/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-2/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-2/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-2/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-3/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-3/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-3/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/baseline-v2-rebuild-samples.jsonl` | no document reference | — |
| `rust-build-efficiency/raw/clean-1/cargo-doctest-list-ignored.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-1/cargo-doctest-list.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-1/cargo-test-list.json` | no document reference | — |
| `rust-build-efficiency/raw/clean-1/cargo-test-list.stderr.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-1/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-1/executable-remeasure.json` | no document reference | mirror source retained by `6eb585bc`; the owning reference names the archived copy |
| `rust-build-efficiency/raw/clean-1/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-2/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-2/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-3/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-3/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/clean-samples.jsonl` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-1/cargo-doctest-list-ignored.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-1/cargo-doctest-list.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-1/cargo-test-list.json` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-1/cargo-test-list.stderr.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-1/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-1/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-2/cargo-test-list.json` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-2/cargo-test-list.stderr.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-2/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-2/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-3/cargo-test-list.json` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-3/cargo-test-list.stderr.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-3/clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-3/test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-clean-samples.jsonl` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-1/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-1/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-1/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-2/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-2/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-2/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-3/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-3/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-3/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/optimized-rebuild-samples.jsonl` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-1/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-1/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-1/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-2/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-2/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-2/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-3/rebuild-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-3/setup-clippy.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-3/setup-test-no-run.log` | no document reference | — |
| `rust-build-efficiency/raw/rebuild-samples.jsonl` | no document reference | — |
| `rust-build-efficiency/report.md` | no document reference | mirror source retained by `6eb585bc`; the owning reference names the archived copy |

## REQ-04 — the container an earlier run already relocated

Container `a9b5dd08` sits at `dev/archive/a9b5dd08`, written by an earlier archival run. It is a
descendant of container `6eb585bc`, one of the 24, so its artifact entered that run's plan — as a
retained row reading `already under the archive root`, the only one of the 33 such rows that lies
inside a mechanism-written container directory.

The run converged on the existing directory. In the tree now, `a9b5dd08-archive-directory-slugs.md`
exists once, at `dev/archive/a9b5dd08/dev/active/a9b5dd08-archive-directory-slugs.md`; nothing under
`dev/archive/6eb585bc-core-maintenance` names it, and the archive root holds no second `a9b5dd08`
directory.

## REQ-05 — citations inside relocated records are unchanged

Two checks, neither of them a re-reading of the evidence the runs wrote:

1. **Byte identity through git.** For each container, the parent of the commit that wrote its marker
   is the pre-run tree. Every destination blob at `HEAD` equals its source blob in that tree, across
   all relocated and mirrored rows (the REQ-01 table). Equal blob ids are equal bytes, so nothing
   inside a relocated record changed, citations included.

2. **Every recorded coordinate still resolves.** Each warning the runs reported carries a
   `line:column`. Reading each citing file at its current path, every occurrence in
   `citation-warning-consolidation.md` matches its cited path at exactly the recorded line and
   column. A rewritten citation, or a rewritten line above one, would move a column or a line.

Archival relocates without rewriting, which `jit doc check-links` makes visible: a relative link
inside a relocated record resolves against the record's depth under the archive root, so a link
written to reach out of its area misses its target. That is the cost of keeping a historical record
verbatim, not a defect of the runs.

## Findings

- **REQ-01's exception clause is discharged by the REQ-03 enumeration.** Every destination the runs
  planned is present and byte-identical; the seven managed-area paths a terminal issue's reference
  still names each carry a recorded reason. Four of them sit in this epic's own subtree, which no
  archived container covers, so the archive root receives them when the epic itself is archived.

- **REQ-03's four reasons do not partition the remaining files by mechanism.** `no document
  reference` covers two populations a reader wants told apart — never-selected artifacts and mirror
  sources whose reference was repointed — so the enumeration marks the latter.

- **`jit doc check-links` exits non-zero on the post-run tree, on findings the runs did not
  introduce.** Its broken-link errors on relocated records follow from leaving content verbatim, as
  above. Its three `missing_document` errors under `.agents/skills/jit-planning-lead/evals/` name
  references that issue `41aa1b75` pins to commit `6b799832`, and all three resolve there, so the
  references hold and the check is what misreads them (`f289ff18`). Its `raw/` error on the
  benchmark report names a directory the mirror did reproduce (`f9e42a43`). Every document reference
  in the repository resolves.

- **Both records sit in the epic's canonical directory, not their own issue's.** The issue requires
  that, while `jit doc dir aa38b236 dev/active` resolves to `dev/active/aa38b236`. `jit doc
  conformance` reports neither of them, and its count is unchanged by linking them: it attributes an
  artifact by the short-id prefix the artifact's own path component carries, and passes over a
  component carrying none, while the containing directory `8e071e18-dev-artifact-layout` is
  `8e071e18`'s canonical directory. The divergence between an artifact's location and its owning
  issue's directory is real and the advisory report does not see it.
