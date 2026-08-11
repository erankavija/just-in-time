# Breakdown Review

Adversarially compare the created breakdown with the authoritative JSON manifest.
Find container `C` from the gated breakdown node's `brackets:` label, locate `P`
and its linked plan/manifest, traverse the full implementation graph, and read
content standards. This is read-only.

Fail a missing manifest; never reconstruct authority from Markdown. Run manifest
validation, renderer `--check`, native batch dry-run, and `jit validate`.

Using the creation key→UUID evidence and stored records, verify:

- exactly one issue per manifest entry, with no missing/extra/merged/split work;
- exact title, standalone description, type, priority, labels, and gates;
- exact intra-manifest edges, neither missing nor over-constraining;
- exact manifest-source→B, C→manifest-sink, and external re-home edges;
- `planning` was not persisted or emitted by batch export;
- every issue meets content standards and every root can start on a blank workspace.

Repeat the finest-tier assignment simulation:

| Key / issue | One outcome | Bounded consumer family | Observable test boundary | Footprint credible | One focused implementation/review cycle | No inner decomposition | No mixed deliverables | Result |
|---|---|---|---|---|---|---|---|---|

Fail a row only when the mis-sizing is material: the worker could not deliver
the issue in one focused cycle, or the scope genuinely mixes deliverable
classes. A borderline row or stylistic sizing concern is an advisory note.
Judge every visible per-code override. A shared `landing_group` or single final
landing never excuses oversized implementation work. Do not recount
criterion labels; the separate coverage gate owns deterministic coverage.

Obey a container decision the plan or manifest explicitly cites; it is settled.
Finding: the graph fails to implement it, or code contradicts its premise.
Not a finding: preferring a different decision, or a shape the cited decision fixes.

Recheck contract modes, unique producer reachability, and all footprint
creates/touches, uncertainties, and overlap advisories against the created graph.

Check `run_history` cumulatively. Superseded prose/issues must be replaced, not
accumulated. A minor issue of a kind that existed unchanged in a previously
reviewed revision is advisory.

## Finding and verdict policy

Every finding must include `disposition` (`blocking` or `advisory`) and
`origin` (`issue-impact` or `pre-existing`). A finding is blocking only if the
created graph as it stands would diverge from the authoritative manifest,
violate a container criterion or a cited invariant, break the dependency graph
or a green intermediate state, lose or weaken semantic coverage, or leave an
issue one worker cannot complete in one focused cycle. Wording and
rationale-text mismatches, redundant-but-harmless edges, citation typos, and
improvable phrasing are advisory. A defect introduced outside this bracket is
`pre-existing` and cannot fail this review. The verdict is `FAIL` if and only
if at least one unresolved issue-impact blocking finding exists. A passing
verdict may therefore contain advisory findings; passing with advisories is a
successful review, not a lenient one. Do not perform an exhaustive
presentation audit.

For each blocking finding give the exact manifest correction and any
graph reconciliation; shared-contract changes must return to plan-review. End
with exactly one line and no following text:

`VERDICT: PASS`

or

`VERDICT: FAIL`
