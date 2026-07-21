# Batch-created bracket spine

Given container `C`, pre-created breakdown node `B`, manifest `M`, and the batch
result's key→UUID map:

1. Compute sources from `M`: entries with empty `depends_on`. Add each mapped
   source's dependency on `B`.
2. Compute sinks: keys no entry names in `depends_on`. Add dependencies from `C`
   to each mapped sink with `jit dep add --reduce <C> <sink...>` so the same
   operation drops the scaffold's now-redundant `C → B` anchor.
3. For every plan Decision that re-homes an external dependency to a semantic
   manifest key, add that external edge to the mapped UUID.
4. Add no other edges. Intra-manifest edges were published by batch-create.
5. Reload every mapped issue and compare native fields and intra-edges to `M`.
   Verify exact source→B, C→sink, and re-home edges, and confirm no `planning`
   field appears in issue storage or batch export.
6. Run the template's coverage gate through the standard gate runner. Block on
   its recorded status. Leave breakdown-review to its configured runner; work
   releases only when both gates pass and `B` is done.

Missing `B`, an unapproved `P`, an absent manifest, incomplete map, or any fidelity
mismatch is a hard stop. Never create bracket nodes or reinterpret Markdown here.
