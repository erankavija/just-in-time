# Plan Review

Adversarially review the planning issue, its container criteria, linked concise
plan, linked authoritative breakdown JSON, investigation sources, dependencies,
and `run_history`. Read actual code and `.jit/reference/content-standards.md`.
This is read-only.

Fail if either artifact is missing, empty, or unlinked. A Markdown-only plan is
not decomposable. Run the manifest helper's validation with warnings denied and
all container criteria required, renderer `--check`, and:

```bash
jit issue batch-create --from-json <manifest> --dry-run --json
```

Any structural/native validation error, cycle, coverage gap, stale generated
region, unresolved sizing warning, or unresolved source/contract reference fails.

Check the plan's architecture, criterion approach, risks, and decisions against
the code. It must be concise: copied issue bodies, exhaustive consumer/file
inventories, repeated acceptance criteria, hand-written task/DAG views, review
history, or append-only correction sections are blocking accumulation.

Check every edge and simulate assigning every finest-tier entry to one worker:

| Key | One outcome | Bounded consumer family | Observable test boundary | One focused implementation/review cycle | No inner decomposition | No mixed foundation/migration/deletion/docs/release work | Result |
|---|---|---|---|---|---|---|---|

Fail any non-passing row. `landing_group` is integration metadata only. Broad
quantifiers, independent verbs, several component families, three acceptance
clusters, and implementation plus release require a split or concrete evidence
of indivisibility. Lack of a finer configured type is never evidence.

Verify prior findings were resolved by replacement and consolidation. Report
blocking findings with artifact/code citations and exact corrections. End with
exactly one line and no following text:

`VERDICT: PASS`

or

`VERDICT: FAIL`
