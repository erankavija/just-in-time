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

| Key / issue | Coherent change | Observable test boundary | Footprint credible | One focused implementation/review cycle | Not a chain link | Result |
|---|---|---|---|---|---|---|

Fail a row only when the mis-sizing is material. Oversize: the worker could not
deliver the issue in one focused cycle, or it combines implementation with
release. Undersize: the issue only links a producer-to-remover chain or a bridge
pair that one task within the module would deliver. Adding a form, moving its
callers, and deleting the old form within one module is one coherent change. A
created graph over the validator's issue budget is blocking. A borderline row or
stylistic sizing concern is an advisory note.
Judge every visible per-code override. A shared `landing_group` or single final
landing never excuses oversized implementation work. Do not recount
criterion labels; the separate coverage gate owns deterministic coverage.

Obey a container decision the plan or manifest explicitly cites; it is settled.
Finding: the graph fails to implement it, or code contradicts its premise.
Not a finding: preferring a different decision, or a shape the cited decision fixes.

Recheck contract modes, unique producer reachability, and all footprint
creates/touches, uncertainties, and overlap advisories against the created graph.

Check `run_history` cumulatively. Superseded prose/issues must be replaced or
merged, not accumulated. A minor issue of a kind that existed unchanged in a previously
reviewed revision is advisory.

## User decisions

The issue description may record user decisions as `decision` items (`D-NN`) under a `## Decisions` heading. Decisions bind this review: treat the state a decision accepts as authoritative, and do not raise a blocking finding whose only remedy the decision forecloses. If evidence contradicts a decision's factual premise, surface that as an advisory finding citing the decision's address, `@/issue/<short-id>/decision/D-NN`.

## Finding and verdict policy

Every finding must include `disposition` (`blocking` or `advisory`) and
`origin` (`issue-impact` or `pre-existing`). A finding is blocking only if the
created graph as it stands would diverge from the authoritative manifest,
violate a container criterion or a cited invariant, break the dependency graph
or a green intermediate state, lose or weaken semantic coverage, leave an
issue one worker cannot complete in one focused cycle, or over-split work into
chain-link issues, bridge producer and remover pairs, or a graph over the issue
budget. Wording and
rationale-text mismatches, redundant-but-harmless edges, citation typos, and
improvable phrasing are advisory. A defect introduced outside this bracket is
`pre-existing` and cannot fail this review. The verdict is `FAIL` if and only
if at least one unresolved issue-impact blocking finding exists. A passing
verdict may therefore contain advisory findings; passing with advisories is a
successful review, not a lenient one. Do not perform an exhaustive
presentation audit.

Report the created issue count and its change from the prior round. For each
blocking finding give the exact manifest correction, replacing or merging
issues, and any graph reconciliation; shared-contract changes must return to plan-review. End
with exactly one line and no following text:

`VERDICT: PASS`

or

`VERDICT: FAIL`
