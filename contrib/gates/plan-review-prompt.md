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

Obey a container decision the plan or manifest explicitly cites; it is settled.
Finding: the plan fails to implement it, or code contradicts its premise.
Not a finding: preferring a different decision, or a shape the cited decision fixes.

Check the plan's architecture, criterion approach, risks, and decisions against
the code. It must be concise: copied issue bodies, exhaustive consumer/file
inventories, repeated acceptance criteria, hand-written task/DAG views, review
history, or append-only correction sections are blocking accumulation.

Check every edge and simulate assigning every finest-tier entry to one worker:

| Key | One outcome | Bounded consumer family | Observable test boundary | Footprint credible | One focused implementation/review cycle | No inner decomposition | No mixed foundation/migration/deletion/docs/release work | Result |
|---|---|---|---|---|---|---|---|---|

Fail a row only when the mis-sizing is material: the worker could not deliver
the leaf in one focused cycle, or the scope genuinely mixes deliverable
classes. A borderline row is an advisory note. `landing_group` is integration
metadata only. Broad quantifiers, independent verbs, several component
families, three acceptance clusters, and implementation plus release require a
split or a concrete per-code override that remains visible here. Reject global
incantations, missing hierarchy/source universes, invented refs, tier-laundered
leaves, and malformed or duplicate contract headings.

Plan-fixed contracts must have no producer. Every implementation-produced
contract must have exactly one producer transitively reachable by each consumer.
Inspect all footprint creates/touches, uncertainties, and overlap advisories;
greenfield work must disclose created paths.

Verify prior findings were resolved by replacement and consolidation. When
`run_history` shows prior review rounds, fail only for regressions, unresolved
prior blocking findings, or new blocking findings under the policy below; a
minor issue of a kind that existed unchanged in a previously reviewed revision
is advisory.

## Finding and verdict policy

Every finding must include `disposition` (`blocking` or `advisory`) and
`origin` (`issue-impact` or `pre-existing`). A finding is blocking only if
instantiating the manifest as written would violate a container criterion or a
cited invariant, break the dependency graph or a green intermediate state,
lose or weaken semantic coverage, or leave a leaf one worker cannot complete
in one focused cycle. Wording and rationale-text mismatches,
redundant-but-harmless edges, citation typos, and improvable phrasing are
advisory. A defect in material the plan merely cites, introduced outside this
bracket, is `pre-existing` and cannot fail this review. The verdict is `FAIL`
if and only if at least one unresolved issue-impact blocking finding exists. A
passing verdict may therefore contain advisory findings; passing with
advisories is a successful review, not a lenient one. Do not perform an
exhaustive presentation audit.

Report findings with artifact/code citations and exact corrections. End with
exactly one line and no following text:

`VERDICT: PASS`

or

`VERDICT: FAIL`
